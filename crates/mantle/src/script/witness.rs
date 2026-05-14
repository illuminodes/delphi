use std::fmt;
use std::io::{Read, Write};

use crate::codec::{Decode, Encode, Error, VarInt, decode_bytes_limited};

// ── WitnessBuf ────────────────────────────────────────────────────────────────

/// Inline-optimised witness stack for a single transaction input (BIP141).
///
/// Stores up to `ITEMS` stack items whose total byte size fits within `BYTES`
/// inline (on the stack, via a boxed array to control enum size); falls back
/// to `Vec<Vec<u8>>` otherwise.
///
/// `ends[i]` is the exclusive end offset of item `i` within `data`. Item `i`
/// occupies `data[ends[i-1]..ends[i]]` (item 0: `data[0..ends[0]]`).
///
/// Wire format (BIP144): `VarInt(count)` followed by each item as
/// `VarInt(len) || bytes`. The witness for each input is written after all
/// outputs in the segwit transaction serialisation; it is absent from the
/// non-witness (txid) serialisation.
#[derive(Clone)]
pub enum WitnessBuf<const ITEMS: usize, const BYTES: usize> {
    /// No stack items — the common case for legacy and non-segwit inputs.
    /// Zero allocation; avoids the `Box<[u8; BYTES]>` that `Inline` requires.
    Empty,
    Inline {
        count: u8,
        ends: [u16; ITEMS],
        used: u16,
        data: Box<[u8; BYTES]>,
    },
    Heap(Vec<Vec<u8>>),
}

/// Iterator over the stack items of a [`WitnessBuf`].
pub struct WitnessIter<'a, const ITEMS: usize, const BYTES: usize> {
    witness: &'a WitnessBuf<ITEMS, BYTES>,
    index: usize,
}

impl<'a, const ITEMS: usize, const BYTES: usize> Iterator for WitnessIter<'a, ITEMS, BYTES> {
    type Item = &'a [u8];

    fn next(&mut self) -> Option<Self::Item> {
        let item = self.witness.get(self.index)?;
        self.index += 1;
        Some(item)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.witness.len().saturating_sub(self.index);
        (remaining, Some(remaining))
    }
}

impl<'a, const ITEMS: usize, const BYTES: usize> IntoIterator for &'a WitnessBuf<ITEMS, BYTES> {
    type Item = &'a [u8];
    type IntoIter = WitnessIter<'a, ITEMS, BYTES>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<const ITEMS: usize, const BYTES: usize> WitnessBuf<ITEMS, BYTES> {
    /// An empty witness (no stack items).
    #[must_use]
    pub fn empty() -> Self {
        Self::Empty
    }

    /// Construct from a slice of byte-string items.
    #[must_use]
    pub fn from_slice(items: &[&[u8]]) -> Self {
        let total: usize = items.iter().map(|i| i.len()).sum();
        if items.len() <= ITEMS && total <= BYTES {
            let mut ends = [0u16; ITEMS];
            let mut data = Box::new([0u8; BYTES]);
            let mut offset = 0usize;
            for (idx, item) in items.iter().enumerate() {
                data[offset..offset + item.len()].copy_from_slice(item);
                offset += item.len();
                #[allow(clippy::cast_possible_truncation)]
                {
                    ends[idx] = offset as u16;
                }
            }
            #[allow(clippy::cast_possible_truncation)]
            Self::Inline {
                count: items.len() as u8,
                ends,
                used: offset as u16,
                data,
            }
        } else {
            Self::Heap(items.iter().map(|i| i.to_vec()).collect())
        }
    }

    /// Number of stack items.
    #[inline]
    #[must_use]
    pub fn len(&self) -> usize {
        match self {
            Self::Empty => 0,
            Self::Inline { count, .. } => *count as usize,
            Self::Heap(v) => v.len(),
        }
    }

    /// `true` if there are no stack items (e.g. a legacy or non-segwit input).
    #[inline]
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Access item `i` by index. Returns `None` if out of bounds.
    #[must_use]
    pub fn get(&self, i: usize) -> Option<&[u8]> {
        match self {
            Self::Empty => None,
            Self::Inline {
                count, ends, data, ..
            } => {
                if i >= *count as usize {
                    return None;
                }
                let start = if i == 0 { 0 } else { ends[i - 1] as usize };
                let end = ends[i] as usize;
                Some(&data[start..end])
            }
            Self::Heap(v) => v.get(i).map(Vec::as_slice),
        }
    }

    /// Iterate over all stack items.
    #[must_use]
    pub fn iter(&self) -> WitnessIter<'_, ITEMS, BYTES> {
        WitnessIter {
            witness: self,
            index: 0,
        }
    }

    /// Materialise as a `Vec<Vec<u8>>`.
    #[must_use]
    pub fn to_vec(&self) -> Vec<Vec<u8>> {
        self.iter().map(<[u8]>::to_vec).collect()
    }

    /// Total byte size of all items (without length prefixes).
    #[must_use]
    pub fn total_bytes(&self) -> usize {
        match self {
            Self::Empty => 0,
            Self::Inline { used, .. } => *used as usize,
            Self::Heap(v) => v.iter().map(Vec::len).sum(),
        }
    }

    /// Encoded byte length for BIP144 witness serialisation of this input's
    /// witness: `VarInt(count)` + per-item `VarInt(len) + bytes`.
    #[must_use]
    pub fn encoded_len(&self) -> usize {
        let count = self.len();
        VarInt::from(count).encoded_len()
            + self
                .iter()
                .map(|item| VarInt::from(item.len()).encoded_len() + item.len())
                .sum::<usize>()
    }
}

impl<const ITEMS: usize, const BYTES: usize> Default for WitnessBuf<ITEMS, BYTES> {
    fn default() -> Self {
        Self::empty()
    }
}

impl<const ITEMS: usize, const BYTES: usize> PartialEq for WitnessBuf<ITEMS, BYTES> {
    fn eq(&self, other: &Self) -> bool {
        if self.len() != other.len() {
            return false;
        }
        self.iter().zip(other.iter()).all(|(a, b)| a == b)
    }
}

impl<const ITEMS: usize, const BYTES: usize> Eq for WitnessBuf<ITEMS, BYTES> {}

impl<const ITEMS: usize, const BYTES: usize> fmt::Debug for WitnessBuf<ITEMS, BYTES> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let items: Vec<_> = self.iter().collect();
        write!(f, "WitnessBuf({items:?})")
    }
}

impl<const ITEMS: usize, const BYTES: usize> Encode for WitnessBuf<ITEMS, BYTES> {
    fn encode(&self, w: &mut impl Write) -> Result<(), Error> {
        VarInt::from(self.len()).encode(w)?;
        for item in self {
            VarInt::from(item.len()).encode(w)?;
            w.write_all(item)?;
        }
        Ok(())
    }
}

impl<const ITEMS: usize, const BYTES: usize> Decode for WitnessBuf<ITEMS, BYTES> {
    fn decode(r: &mut impl Read) -> Result<Self, Error> {
        let count = VarInt::decode(r)?.to_usize()?;
        let mut items: Vec<Vec<u8>> = Vec::with_capacity(count.min(16));
        for _ in 0..count {
            // Witness stack items have no per-item consensus size limit (unlike
            // script push data). Cap at 4 MiB — the block weight bound.
            let item = decode_bytes_limited::<{ 4 * 1024 * 1024 }>(r)?;
            items.push(item);
        }
        let slices: Vec<&[u8]> = items.iter().map(Vec::as_slice).collect();
        Ok(Self::from_slice(&slices))
    }
}

/// Witness stack for a transaction input. Inline up to 4 items / 164 bytes.
///
/// 4 items covers P2WPKH (2), most P2WSH multisig (≤4), and taproot key-path
/// (1, or 2 with annex). 164 bytes covers a P2WPKH witness (sig 73 + pubkey
/// 33 = 106) with room for P2WSH scripts up to ~58 bytes inline.
pub type Witness = WitnessBuf<4, 164>;
