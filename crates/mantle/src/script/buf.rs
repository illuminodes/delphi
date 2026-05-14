use std::fmt;
use std::io::{Read, Write};

use crate::codec::{Decode, Encode, Error, VarInt, decode_bytes_limited};

// ── ScriptBuf ─────────────────────────────────────────────────────────────────

/// Inline-optimised byte buffer for Bitcoin scripts.
///
/// Stores scripts up to `N` bytes on the stack; falls back to heap for longer
/// ones. `N` is chosen per field: 128 for `ScriptSig` (covers all coinbase and
/// standard legacy inputs), 36 for `ScriptPubKey` (covers every standard output
/// type including P2WSH at 34 bytes).
#[derive(Clone)]
pub enum ScriptBuf<const N: usize> {
    Inline { len: u8, data: [u8; N] },
    Heap(Vec<u8>),
}

impl<const N: usize> ScriptBuf<N> {
    /// Construct from a byte slice, choosing inline or heap automatically.
    ///
    /// # Panics
    /// Never in practice — `N` is always ≤ 255 (128 or 36), so `s.len() <= N`
    /// implies `s.len()` fits in `u8`.
    #[must_use]
    pub fn from_slice(s: &[u8]) -> Self {
        if s.len() <= N {
            let mut data = [0u8; N];
            data[..s.len()].copy_from_slice(s);
            #[allow(clippy::cast_possible_truncation)] // N <= 255, so len <= N fits u8
            Self::Inline {
                len: s.len() as u8,
                data,
            }
        } else {
            Self::Heap(s.to_vec())
        }
    }

    #[inline]
    #[must_use]
    pub fn as_slice(&self) -> &[u8] {
        match self {
            Self::Inline { len, data } => &data[..*len as usize],
            Self::Heap(v) => v.as_slice(),
        }
    }

    #[inline]
    #[must_use]
    pub fn len(&self) -> usize {
        match self {
            Self::Inline { len, .. } => *len as usize,
            Self::Heap(v) => v.len(),
        }
    }

    #[inline]
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    #[must_use]
    pub fn to_vec(&self) -> Vec<u8> {
        self.as_slice().to_vec()
    }
}

impl<const N: usize> std::ops::Deref for ScriptBuf<N> {
    type Target = [u8];
    #[inline]
    fn deref(&self) -> &[u8] {
        self.as_slice()
    }
}

impl<const N: usize> From<Vec<u8>> for ScriptBuf<N> {
    fn from(v: Vec<u8>) -> Self {
        if v.len() <= N {
            Self::from_slice(&v)
        } else {
            Self::Heap(v)
        }
    }
}

impl<const N: usize> From<&[u8]> for ScriptBuf<N> {
    fn from(s: &[u8]) -> Self {
        Self::from_slice(s)
    }
}

impl<const N: usize> PartialEq for ScriptBuf<N> {
    fn eq(&self, other: &Self) -> bool {
        self.as_slice() == other.as_slice()
    }
}

impl<const N: usize> Eq for ScriptBuf<N> {}

impl<const N: usize> PartialEq<[u8]> for ScriptBuf<N> {
    fn eq(&self, other: &[u8]) -> bool {
        self.as_slice() == other
    }
}

impl<const N: usize> PartialEq<Vec<u8>> for ScriptBuf<N> {
    fn eq(&self, other: &Vec<u8>) -> bool {
        self.as_slice() == other.as_slice()
    }
}

impl<const N: usize> fmt::Debug for ScriptBuf<N> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ScriptBuf({:?})", self.as_slice())
    }
}

impl<const N: usize> Encode for ScriptBuf<N> {
    fn encode(&self, w: &mut impl Write) -> Result<(), Error> {
        let s = self.as_slice();
        VarInt::from(s.len()).encode(w)?;
        w.write_all(s)?;
        Ok(())
    }
}

impl<const N: usize> Decode for ScriptBuf<N> {
    fn decode(r: &mut impl Read) -> Result<Self, Error> {
        decode_bytes_limited::<10_000>(r).map(Self::from)
    }
}

// ── ScriptSig / ScriptPubKey ──────────────────────────────────────────────────

/// Input script (scriptSig). Inline up to 128 bytes; heap beyond that.
///
/// 128 bytes covers all standard legacy inputs (P2PKH scriptSig is 107 bytes)
/// and the consensus coinbase maximum (100 bytes).
pub type ScriptSig = ScriptBuf<128>;

/// Output script (scriptPubKey). Inline up to 36 bytes; heap beyond that.
///
/// 36 bytes covers every standard output type: P2PKH (25), P2SH (23),
/// P2WPKH (22), P2WSH (34), `OP_RETURN` bare (1+).
pub type ScriptPubKey = ScriptBuf<36>;
