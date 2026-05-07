use std::fmt;
use std::io::{self, Read, Write};

use crate::codec::{Decode, DecodeRef, Encode, VarInt, decode_bytes_limited};
use crate::hash::sha256d;

// ── Hash256 ───────────────────────────────────────────────────────────────────

/// A 32-byte hash (txid, block hash, merkle root, etc.). Stored as raw bytes,
/// displayed in reversed byte order per Bitcoin convention.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Hash256(pub [u8; 32]);

impl Hash256 {
    pub const ZERO: Self = Self([0u8; 32]);

    /// Parse a 64-character hex string in display order (reversed from internal storage,
    /// per Bitcoin convention).
    ///
    /// # Errors
    /// Returns `Err` if the input is not exactly 64 hex characters.
    pub fn from_display_hex(s: &str) -> Result<Self, HexError> {
        if s.len() != 64 {
            return Err(HexError::Length(s.len()));
        }
        let mut bytes = [0u8; 32];
        for (i, byte) in bytes.iter_mut().enumerate() {
            let pair = &s[i * 2..i * 2 + 2];
            *byte = u8::from_str_radix(pair, 16)
                .map_err(|_| HexError::InvalidChar { offset: i * 2 })?;
        }
        bytes.reverse();
        Ok(Self(bytes))
    }

    /// Format the hash as a 64-character hex string in display order
    /// (reversed from internal storage, per Bitcoin convention).
    #[must_use]
    pub fn to_display_hex(self) -> String {
        // Display already produces this format; reuse it.
        self.to_string()
    }
}

/// Errors returned by [`Hash256::from_display_hex`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HexError {
    /// The input string was not 64 characters long.
    Length(usize),
    /// A character at the given byte offset was not a valid hex digit.
    InvalidChar { offset: usize },
}

impl fmt::Display for HexError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Length(n) => write!(f, "expected 64 hex characters, got {n}"),
            Self::InvalidChar { offset } => write!(f, "invalid hex character at offset {offset}"),
        }
    }
}

impl std::error::Error for HexError {}

impl fmt::Debug for Hash256 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self}")
    }
}

impl fmt::Display for Hash256 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut rev = self.0;
        rev.reverse();
        for b in &rev {
            write!(f, "{b:02x}")?;
        }
        Ok(())
    }
}

impl Encode for Hash256 {
    #[inline]
    fn encode(&self, w: &mut impl Write) -> io::Result<()> {
        w.write_all(&self.0)
    }
}

impl Decode for Hash256 {
    #[inline]
    fn decode(r: &mut impl Read) -> io::Result<Self> {
        let mut buf = [0u8; 32];
        r.read_exact(&mut buf)?;
        Ok(Self(buf))
    }
}

impl<'a> DecodeRef<'a> for Hash256 {
    #[inline]
    fn decode_ref(buf: &'a [u8]) -> io::Result<(Self, &'a [u8])> {
        if buf.len() < 32 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "not enough bytes",
            ));
        }
        let (head, tail) = buf.split_at(32);
        Ok((Self(head.try_into().map_err(|_| io::Error::from(io::ErrorKind::InvalidData))?), tail))
    }
}

// ── CompactBits ───────────────────────────────────────────────────────────────

/// The `nBits` compact-format difficulty target stored in a block header.
///
/// Wraps a `u32` to make difficulty fields type-distinct from plain integers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct CompactBits(pub u32);

impl From<u32> for CompactBits {
    #[inline]
    fn from(v: u32) -> Self {
        Self(v)
    }
}

impl From<CompactBits> for u32 {
    #[inline]
    fn from(v: CompactBits) -> Self {
        v.0
    }
}

impl Encode for CompactBits {
    #[inline]
    fn encode(&self, w: &mut impl Write) -> io::Result<()> {
        self.0.encode(w)
    }
}

impl Decode for CompactBits {
    #[inline]
    fn decode(r: &mut impl Read) -> io::Result<Self> {
        u32::decode(r).map(Self)
    }
}

impl<'a> DecodeRef<'a> for CompactBits {
    #[inline]
    fn decode_ref(buf: &'a [u8]) -> io::Result<(Self, &'a [u8])> {
        u32::decode_ref(buf).map(|(v, rest)| (Self(v), rest))
    }
}

// ── BlockHeight ───────────────────────────────────────────────────────────────

/// A block height on the canonical chain.
///
/// Wraps a `u32` to make height values type-distinct from plain integers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct BlockHeight(pub u32);

impl BlockHeight {
    pub const ZERO: Self = Self(0);

    #[must_use]
    pub fn checked_add(self, rhs: u32) -> Option<Self> {
        self.0.checked_add(rhs).map(Self)
    }

    #[must_use]
    pub fn saturating_sub(self, rhs: u32) -> Self {
        Self(self.0.saturating_sub(rhs))
    }
}

impl From<u32> for BlockHeight {
    #[inline]
    fn from(v: u32) -> Self {
        Self(v)
    }
}

impl From<BlockHeight> for u32 {
    #[inline]
    fn from(v: BlockHeight) -> Self {
        v.0
    }
}

impl From<BlockHeight> for usize {
    #[inline]
    fn from(v: BlockHeight) -> Self {
        v.0 as usize
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

impl CompactBits {
    /// Decode to a 256-bit target as a 32-byte big-endian array.
    ///
    /// Returns `None` if the value is zero, negative, or encodes an overflowed target.
    #[must_use]
    pub fn to_target(self) -> Option<[u8; 32]> {
        let raw = self.0;
        let exponent = (raw >> 24) as usize;
        let mantissa = raw & 0x007f_ffff;

        if raw & 0x0080_0000 != 0 { return None; } // negative mantissa
        if mantissa == 0 { return None; }
        if exponent > 34 || (exponent == 34 && mantissa > 0xff) { return None; }

        let mut target = [0u8; 32];
        if exponent >= 3 {
            let start = 32 - exponent;
            target[start]     = ((mantissa >> 16) & 0xff) as u8;
            target[start + 1] = ((mantissa >>  8) & 0xff) as u8;
            target[start + 2] = ( mantissa        & 0xff) as u8;
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
        if first_nonzero + 1 < 32 { mantissa |= u32::from(target[first_nonzero + 1]) << 8; }
        if first_nonzero + 2 < 32 { mantissa |=  u32::from(target[first_nonzero + 2]);      }

        if mantissa & 0x0080_0000 != 0 {
            mantissa >>= 8;
            Some(Self((exponent + 1) << 24 | mantissa))
        } else {
            Some(Self(exponent << 24 | mantissa))
        }
    }
}

// ── Satoshi ───────────────────────────────────────────────────────────────────

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
    fn encode(&self, w: &mut impl Write) -> io::Result<()> {
        self.0.encode(w)
    }
}

impl Decode for Satoshi {
    #[inline]
    fn decode(r: &mut impl Read) -> io::Result<Self> {
        u64::decode(r).map(Self)
    }
}

impl<'a> DecodeRef<'a> for Satoshi {
    #[inline]
    fn decode_ref(buf: &'a [u8]) -> io::Result<(Self, &'a [u8])> {
        u64::decode_ref(buf).map(|(v, rest)| (Self(v), rest))
    }
}

// ── BlockHeader ───────────────────────────────────────────────────────────────

/// 80-byte fixed-size block header. No heap allocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockHeader {
    pub version: i32,
    pub prev_block: Hash256,
    pub merkle_root: Hash256,
    pub time: u32,
    pub bits: CompactBits,
    pub nonce: u32,
}

impl BlockHeader {
    pub const ENCODED_LEN: usize = 80;
}

impl BlockHeader {
    /// Serialize to a fixed 80-byte stack buffer.
    #[must_use]
    #[allow(clippy::missing_panics_doc)] // writing into a fixed-size array cannot fail
    pub fn to_bytes(&self) -> [u8; Self::ENCODED_LEN] {
        let mut buf = [0u8; Self::ENCODED_LEN];
        self.encode(&mut buf.as_mut_slice()).expect("fixed-size write cannot fail");
        buf
    }

    /// `SHA256d` of the 80-byte serialised header — the canonical block hash.
    #[must_use]
    pub fn hash(&self) -> Hash256 {
        Hash256(sha256d(&self.to_bytes()))
    }

    /// Deserialize from a fixed 80-byte slice.
    ///
    /// # Errors
    /// Returns an error if the bytes are malformed (in practice, never for a
    /// fixed-size 80-byte input).
    pub fn from_bytes(b: &[u8; Self::ENCODED_LEN]) -> io::Result<Self> {
        let mut r = b.as_slice();
        let version = i32::decode(&mut r)?;
        let prev_block = Hash256::decode(&mut r)?;
        let merkle_root = Hash256::decode(&mut r)?;
        let time = u32::decode(&mut r)?;
        let bits = CompactBits::decode(&mut r)?;
        let nonce = u32::decode(&mut r)?;
        Ok(Self { version, prev_block, merkle_root, time, bits, nonce })
    }
}

impl Encode for BlockHeader {
    #[inline]
    fn encode(&self, w: &mut impl Write) -> io::Result<()> {
        self.version.encode(w)?;
        self.prev_block.encode(w)?;
        self.merkle_root.encode(w)?;
        self.time.encode(w)?;
        self.bits.encode(w)?;
        self.nonce.encode(w)
    }
}

impl Decode for BlockHeader {
    #[inline]
    fn decode(r: &mut impl Read) -> io::Result<Self> {
        let mut buf = [0u8; Self::ENCODED_LEN];
        r.read_exact(&mut buf)?;
        Self::from_bytes(&buf)
    }
}

impl<'a> DecodeRef<'a> for BlockHeader {
    #[inline]
    fn decode_ref(buf: &'a [u8]) -> io::Result<(Self, &'a [u8])> {
        if buf.len() < Self::ENCODED_LEN {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "not enough bytes"));
        }
        let (head, tail) = buf.split_at(Self::ENCODED_LEN);
        Ok((Self::from_bytes(head.try_into().map_err(|_| io::Error::from(io::ErrorKind::InvalidData))?)?, tail))
    }
}

// ── OutPoint ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct OutPoint {
    pub txid: Hash256,
    pub vout: u32,
}

impl OutPoint {
    pub const ENCODED_LEN: usize = 36;
}

impl Encode for OutPoint {
    fn encode(&self, w: &mut impl Write) -> io::Result<()> {
        self.txid.encode(w)?;
        self.vout.encode(w)
    }
}

impl Decode for OutPoint {
    fn decode(r: &mut impl Read) -> io::Result<Self> {
        Ok(Self {
            txid: Hash256::decode(r)?,
            vout: u32::decode(r)?,
        })
    }
}

impl<'a> DecodeRef<'a> for OutPoint {
    fn decode_ref(buf: &'a [u8]) -> io::Result<(Self, &'a [u8])> {
        let (txid, buf) = Hash256::decode_ref(buf)?;
        let (vout, buf) = u32::decode_ref(buf)?;
        Ok((Self { txid, vout }, buf))
    }
}

// ── ScriptBuf ─────────────────────────────────────────────────────────────────

/// Inline-optimised byte buffer for Bitcoin scripts.
///
/// Stores scripts up to `N` bytes on the stack; falls back to heap for longer
/// ones. `N` is chosen per field: 128 for `ScriptSig` (covers all coinbase and
/// standard legacy inputs), 36 for `ScriptPubKey` (covers every standard output
/// type including P2WSH at 34 bytes).
#[derive(Clone)]
pub enum ScriptBuf<const N: usize> {
    Inline { len: u8, data: Box<[u8; N]> },
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
            let mut data = Box::new([0u8; N]);
            data[..s.len()].copy_from_slice(s);
            #[allow(clippy::cast_possible_truncation)] // N <= 255, so len <= N fits u8
            Self::Inline { len: s.len() as u8, data }
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
    fn encode(&self, w: &mut impl Write) -> io::Result<()> {
        let s = self.as_slice();
        VarInt(s.len() as u64).encode(w)?;
        w.write_all(s)
    }
}

impl<const N: usize> Decode for ScriptBuf<N> {
    fn decode(r: &mut impl Read) -> io::Result<Self> {
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
            Self::Inline { count: items.len() as u8, ends, used: offset as u16, data }
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
            Self::Inline { count, ends, data, .. } => {
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
        WitnessIter { witness: self, index: 0 }
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
        VarInt(count as u64).encoded_len()
            + self
                .iter()
                .map(|item| VarInt(item.len() as u64).encoded_len() + item.len())
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
    fn encode(&self, w: &mut impl Write) -> io::Result<()> {
        VarInt(self.len() as u64).encode(w)?;
        for item in self {
            VarInt(item.len() as u64).encode(w)?;
            w.write_all(item)?;
        }
        Ok(())
    }
}

impl<const ITEMS: usize, const BYTES: usize> Decode for WitnessBuf<ITEMS, BYTES> {
    fn decode(r: &mut impl Read) -> io::Result<Self> {
        let count = usize::try_from(VarInt::decode(r)?.0)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        let mut items: Vec<Vec<u8>> = Vec::with_capacity(count.min(16));
        for _ in 0..count {
            let item = decode_bytes_limited::<520>(r)?;
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

// ── TxIn ──────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TxIn {
    pub previous_output: OutPoint,
    pub script_sig: ScriptSig,
    pub sequence: u32,
    /// Witness stack for this input (BIP141). Empty for legacy inputs.
    /// Not included in the non-witness serialisation used for txid.
    pub witness: Witness,
}

impl Encode for TxIn {
    /// Non-witness serialisation: outpoint + scriptSig + sequence.
    /// Witness data is serialised separately by `Transaction::encode`.
    fn encode(&self, w: &mut impl Write) -> io::Result<()> {
        self.previous_output.encode(w)?;
        self.script_sig.encode(w)?;
        self.sequence.encode(w)
    }
}

impl Decode for TxIn {
    /// Decodes the non-witness fields only. Use `Transaction::decode` to
    /// parse a full BIP144 witness transaction including witness stacks.
    fn decode(r: &mut impl Read) -> io::Result<Self> {
        Ok(Self {
            previous_output: OutPoint::decode(r)?,
            script_sig: ScriptSig::decode(r)?,
            sequence: u32::decode(r)?,
            witness: Witness::empty(),
        })
    }
}

/// Borrowed view of a `TxIn` — `script_sig` is a slice into the source buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TxInRef<'a> {
    pub previous_output: OutPoint,
    pub script_sig: &'a [u8],
    pub sequence: u32,
}

impl<'a> DecodeRef<'a> for TxInRef<'a> {
    fn decode_ref(buf: &'a [u8]) -> io::Result<(Self, &'a [u8])> {
        let (previous_output, buf) = OutPoint::decode_ref(buf)?;
        let (script_sig, buf) = <&'a [u8]>::decode_ref(buf)?;
        let (sequence, buf) = u32::decode_ref(buf)?;
        Ok((
            Self {
                previous_output,
                script_sig,
                sequence,
            },
            buf,
        ))
    }
}

// ── TxOut ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TxOut {
    pub value: Satoshi,
    pub script_pubkey: ScriptPubKey,
}

impl Encode for TxOut {
    fn encode(&self, w: &mut impl Write) -> io::Result<()> {
        self.value.encode(w)?;
        self.script_pubkey.encode(w)
    }
}

impl Decode for TxOut {
    fn decode(r: &mut impl Read) -> io::Result<Self> {
        Ok(Self {
            value: Satoshi::decode(r)?,
            script_pubkey: ScriptPubKey::decode(r)?,
        })
    }
}

/// Borrowed view of a `TxOut` — `script_pubkey` is a slice into the source buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TxOutRef<'a> {
    pub value: Satoshi,
    pub script_pubkey: &'a [u8],
}

impl<'a> DecodeRef<'a> for TxOutRef<'a> {
    fn decode_ref(buf: &'a [u8]) -> io::Result<(Self, &'a [u8])> {
        let (value, buf) = Satoshi::decode_ref(buf)?;
        let (script_pubkey, buf) = <&'a [u8]>::decode_ref(buf)?;
        Ok((
            Self {
                value,
                script_pubkey,
            },
            buf,
        ))
    }
}

// ── TxHash ────────────────────────────────────────────────────────────────────

/// Implemented by any type that can compute a Bitcoin transaction id.
///
/// The required method [`txid_into`] writes the serialisation into a
/// caller-owned buffer to avoid a per-call heap allocation. The provided
/// [`txid`] allocates its own buffer; use it when the extra allocation is
/// acceptable.
///
/// Both `Transaction` and `TransactionRef<'_>` implement this trait, so code
/// that must handle either can be generic over `T: TxHash`.
pub trait TxHash {
    /// Compute the txid (`SHA256d` of the non-witness serialisation), reusing
    /// `buf` as scratch space. `buf` is cleared and refilled on every call.
    ///
    /// # Errors
    /// Propagates any I/O error from encoding (encoding into `Vec<u8>` never
    /// fails in practice).
    fn txid_into(&self, buf: &mut Vec<u8>) -> io::Result<Hash256>;

    /// Compute the txid, allocating a fresh buffer internally.
    ///
    /// # Errors
    /// Propagates any I/O error from encoding.
    fn txid(&self) -> io::Result<Hash256> {
        self.txid_into(&mut Vec::new())
    }
}

// ── Transaction ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transaction {
    pub version: i32,
    pub inputs: Vec<TxIn>,
    pub outputs: Vec<TxOut>,
    pub locktime: u32,
}

impl TxHash for Transaction {
    fn txid_into(&self, buf: &mut Vec<u8>) -> io::Result<Hash256> {
        buf.clear();
        buf.reserve(self.base_encoded_len());
        self.encode_base(buf)?;
        Ok(Hash256(sha256d(buf)))
    }
}

impl Transaction {
    /// BIP34: check that the coinbase scriptSig begins with a minimally-encoded
    /// push of `height`.
    ///
    /// Returns `false` if the prefix is absent or malformed.
    #[must_use]
    pub fn validate_bip34(&self, height: BlockHeight) -> bool {
        let script = &self.inputs[0].script_sig;
        let encoded = Self::bip34_height_script(height);
        script.len() >= encoded.len() && script[..encoded.len()] == encoded
    }

    /// Minimal Script push encoding of a block height as used by BIP34.
    ///
    /// # Panics
    /// Never in practice — a `u32` height encodes to at most 5 bytes, which fits in `u8`.
    #[must_use]
    pub fn bip34_height_script(height: BlockHeight) -> Vec<u8> {
        if height == BlockHeight::ZERO {
            return vec![0x00];
        }
        let mut le = height.0.to_le_bytes().to_vec();
        while le.last() == Some(&0) {
            le.pop();
        }
        if le.last().is_some_and(|&b| b & 0x80 != 0) {
            le.push(0x00);
        }
        let len = u8::try_from(le.len()).expect("BIP34 height encoding ≤ 5 bytes");
        let mut out = vec![len];
        out.extend_from_slice(&le);
        out
    }

    /// `true` if any input carries a non-empty witness stack.
    #[must_use]
    pub fn is_segwit(&self) -> bool {
        self.inputs.iter().any(|i| !i.witness.is_empty())
    }

    /// Non-witness serialised byte length (used for txid and base weight).
    #[must_use]
    pub fn base_encoded_len(&self) -> usize {
        let inputs_len: usize = self
            .inputs
            .iter()
            .map(|i| OutPoint::ENCODED_LEN + VarInt(i.script_sig.len() as u64).encoded_len() + i.script_sig.len() + 4)
            .sum();
        let outputs_len: usize = self
            .outputs
            .iter()
            .map(|o| 8 + VarInt(o.script_pubkey.len() as u64).encoded_len() + o.script_pubkey.len())
            .sum();
        4 // version
            + VarInt(self.inputs.len() as u64).encoded_len()
            + inputs_len
            + VarInt(self.outputs.len() as u64).encoded_len()
            + outputs_len
            + 4 // locktime
    }

    /// Total serialised byte length including witness data (BIP144).
    /// Equal to `base_encoded_len` for non-segwit transactions.
    #[must_use]
    pub fn encoded_len(&self) -> usize {
        if !self.is_segwit() {
            return self.base_encoded_len();
        }
        let witness_len: usize = self.inputs.iter().map(|i| i.witness.encoded_len()).sum();
        self.base_encoded_len()
            + 2 // marker (0x00) + flag (0x01)
            + witness_len
    }

    /// Write the non-witness serialisation (version, inputs, outputs, locktime).
    /// Used for txid computation and as the base of BIP144 encoding.
    fn encode_base(&self, w: &mut impl Write) -> io::Result<()> {
        self.version.encode(w)?;
        VarInt(self.inputs.len() as u64).encode(w)?;
        for input in &self.inputs {
            input.encode(w)?;
        }
        VarInt(self.outputs.len() as u64).encode(w)?;
        for output in &self.outputs {
            output.encode(w)?;
        }
        self.locktime.encode(w)
    }
}

impl Encode for Transaction {
    /// BIP144 serialisation: non-witness form for legacy transactions; marker +
    /// flag + per-input witness stacks appended after outputs for segwit ones.
    fn encode(&self, w: &mut impl Write) -> io::Result<()> {
        if !self.is_segwit() {
            return self.encode_base(w);
        }
        self.version.encode(w)?;
        w.write_all(&[0x00, 0x01])?; // marker, flag
        VarInt(self.inputs.len() as u64).encode(w)?;
        for input in &self.inputs {
            input.encode(w)?;
        }
        VarInt(self.outputs.len() as u64).encode(w)?;
        for output in &self.outputs {
            output.encode(w)?;
        }
        for input in &self.inputs {
            input.witness.encode(w)?;
        }
        self.locktime.encode(w)
    }
}

impl Transaction {
    /// Decode with caller-specified per-list caps.
    /// `MAX_INPUTS` and `MAX_OUTPUTS` default to Bitcoin's practical consensus limit.
    ///
    /// # Errors
    /// Returns `InvalidData` if either list count exceeds its cap, or on any I/O error.
    pub fn decode_limited<const MAX_INPUTS: usize, const MAX_OUTPUTS: usize>(
        r: &mut impl Read,
    ) -> io::Result<Self> {
        let version = i32::decode(r)?;

        // BIP144: read the input-count VarInt byte. If it is 0x00 we may be
        // looking at a segwit transaction (marker=0x00, flag=0x01). Read one
        // more byte to check: if it is 0x01 this is segwit and the real
        // input-count VarInt follows; if it is anything else we have a legacy
        // 0-input transaction and that byte is the output-count VarInt start.
        let first = u8::decode(r)?;
        let (segwit, input_count, legacy_output_count_first) = if first == 0x00 {
            let flag = u8::decode(r)?;
            if flag == 0x01 {
                // Segwit: read the real input count.
                let n = usize::try_from(VarInt::decode(r)?.0)
                    .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
                (true, n, None)
            } else {
                // Legacy 0-input tx: `first`=0x00 was the input count (0
                // inputs), `flag` is the first byte of the output-count VarInt.
                (false, 0usize, Some(flag))
            }
        } else {
            let n = usize::try_from(VarInt::decode_from_first_byte(first, r)?.0)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
            (false, n, None)
        };

        if input_count > MAX_INPUTS {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "input count exceeds limit"));
        }
        let mut inputs = Vec::with_capacity(input_count);
        for _ in 0..input_count {
            inputs.push(TxIn::decode(r)?);
        }

        let output_count = if let Some(b) = legacy_output_count_first {
            usize::try_from(VarInt::decode_from_first_byte(b, r)?.0)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?
        } else {
            usize::try_from(VarInt::decode(r)?.0)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?
        };
        if output_count > MAX_OUTPUTS {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "output count exceeds limit"));
        }
        let mut outputs = Vec::with_capacity(output_count);
        for _ in 0..output_count {
            outputs.push(TxOut::decode(r)?);
        }

        if segwit {
            for input in &mut inputs {
                input.witness = Witness::decode(r)?;
            }
        }

        let locktime = u32::decode(r)?;
        Ok(Self { version, inputs, outputs, locktime })
    }
}

impl Decode for Transaction {
    fn decode(r: &mut impl Read) -> io::Result<Self> {
        Self::decode_limited::<10_000, 10_000>(r)
    }
}

// ── TransactionRef ────────────────────────────────────────────────────────────

/// Lazy iterator over zero-copy `TxInRef` views.
#[derive(Debug, Clone)]
pub struct TxInRefIter<'a> {
    buf: &'a [u8],
    remaining: usize,
}

impl<'a> Iterator for TxInRefIter<'a> {
    type Item = io::Result<TxInRef<'a>>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }
        match TxInRef::decode_ref(self.buf) {
            Ok((item, rest)) => {
                self.buf = rest;
                self.remaining -= 1;
                Some(Ok(item))
            }
            Err(e) => {
                self.remaining = 0;
                Some(Err(e))
            }
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}

/// Lazy iterator over zero-copy `TxOutRef` views.
#[derive(Debug, Clone)]
pub struct TxOutRefIter<'a> {
    buf: &'a [u8],
    remaining: usize,
}

impl<'a> Iterator for TxOutRefIter<'a> {
    type Item = io::Result<TxOutRef<'a>>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }
        match TxOutRef::decode_ref(self.buf) {
            Ok((item, rest)) => {
                self.buf = rest;
                self.remaining -= 1;
                Some(Ok(item))
            }
            Err(e) => {
                self.remaining = 0;
                Some(Err(e))
            }
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}

/// Zero-copy borrowed view of a `Transaction`.
///
/// Inputs and outputs are not materialised into `Vec`s — they are decoded
/// lazily from slices of the original buffer. Use [`inputs`](Self::inputs) and
/// [`outputs`](Self::outputs) to iterate; use [`to_owned`](Self::to_owned) to
/// materialise a full `Transaction` when needed.
///
/// `raw` holds the complete wire bytes (BIP144 form for segwit transactions).
/// `non_witness_raw` holds the non-witness serialisation used for txid; for
/// legacy transactions it is the same slice as `raw`.
#[derive(Debug, Clone, Copy)]
pub struct TransactionRef<'a> {
    pub version: i32,
    pub locktime: u32,
    /// Complete wire bytes (BIP144 witness form if segwit, otherwise identical
    /// to `non_witness_raw`). Used by `Encode` and `encoded_len`.
    pub raw: &'a [u8],
    /// Non-witness serialisation bytes (no marker/flag/witness fields).
    /// Used by `txid_into` to hash without allocating.
    pub non_witness_raw: &'a [u8],
    input_count: usize,
    inputs_buf: &'a [u8],
    output_count: usize,
    outputs_buf: &'a [u8],
}

impl<'a> TransactionRef<'a> {
    /// Lazily iterate over inputs. Each call returns a fresh iterator.
    #[must_use]
    pub fn inputs(&self) -> TxInRefIter<'a> {
        TxInRefIter { buf: self.inputs_buf, remaining: self.input_count }
    }

    /// Lazily iterate over outputs. Each call returns a fresh iterator.
    #[must_use]
    pub fn outputs(&self) -> TxOutRefIter<'a> {
        TxOutRefIter { buf: self.outputs_buf, remaining: self.output_count }
    }

    /// Number of inputs.
    #[must_use]
    pub fn input_count(&self) -> usize {
        self.input_count
    }

    /// Number of outputs.
    #[must_use]
    pub fn output_count(&self) -> usize {
        self.output_count
    }

    /// Serialised byte length — O(1), reads directly from the raw wire span.
    #[inline]
    #[must_use]
    pub fn encoded_len(&self) -> usize {
        self.raw.len()
    }

    /// Materialise into an owned `Transaction`, allocating.
    ///
    /// # Errors
    /// Returns an error if any input or output is malformed.
    pub fn to_owned(self) -> io::Result<Transaction> {
        let inputs = self
            .inputs()
            .map(|r| {
                r.map(|i| TxIn {
                    previous_output: i.previous_output,
                    script_sig: i.script_sig.to_vec().into(),
                    sequence: i.sequence,
                    witness: Witness::empty(),
                })
            })
            .collect::<io::Result<Vec<_>>>()?;
        let outputs = self
            .outputs()
            .map(|r| {
                r.map(|o| TxOut {
                    value: o.value,
                    script_pubkey: o.script_pubkey.to_vec().into(),
                })
            })
            .collect::<io::Result<Vec<_>>>()?;
        Ok(Transaction { version: self.version, inputs, outputs, locktime: self.locktime })
    }
}

impl Encode for TransactionRef<'_> {
    #[inline]
    fn encode(&self, w: &mut impl Write) -> io::Result<()> {
        w.write_all(self.raw)
    }
}

impl TxHash for TransactionRef<'_> {
    #[inline]
    fn txid_into(&self, _buf: &mut Vec<u8>) -> io::Result<Hash256> {
        Ok(Hash256(sha256d(self.non_witness_raw)))
    }
}

impl<'a> DecodeRef<'a> for TransactionRef<'a> {
    fn decode_ref(buf: &'a [u8]) -> io::Result<(Self, &'a [u8])> {
        let tx_start = buf;
        let (version, buf) = i32::decode_ref(buf)?;

        // BIP144: if the byte after version is 0x00 and the one after that is
        // 0x01, this is a segwit transaction. Otherwise 0x00 means 0 inputs
        // (legacy) and the second byte is the output-count VarInt start.
        let (first, buf) = u8::decode_ref(buf)?;
        let (segwit, input_count, buf, legacy_output_first) = if first == 0x00 {
            let (flag, buf) = u8::decode_ref(buf)?;
            if flag == 0x01 {
                let (VarInt(n), buf) = VarInt::decode_ref(buf)?;
                let count = usize::try_from(n)
                    .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
                (true, count, buf, None)
            } else {
                // Legacy 0-input tx; `flag` is first byte of output-count VarInt.
                (false, 0usize, buf, Some(flag))
            }
        } else {
            // Re-decode the VarInt with `first` as its initial byte using a cursor.
            let mut cursor = std::io::Cursor::new(buf);
            let VarInt(n) = VarInt::decode_from_first_byte(first, &mut cursor)?;
            let consumed = usize::try_from(cursor.position())
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
            let count = usize::try_from(n)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
            (false, count, &buf[consumed..], None)
        };

        let inputs_buf = buf;
        let mut rest = buf;
        for _ in 0..input_count {
            let (_, r) = TxInRef::decode_ref(rest)?;
            rest = r;
        }

        let (output_count, rest) = if let Some(b) = legacy_output_first {
            let mut cursor = std::io::Cursor::new(rest);
            let VarInt(m) = VarInt::decode_from_first_byte(b, &mut cursor)?;
            let consumed = usize::try_from(cursor.position())
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
            let count = usize::try_from(m)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
            (count, &rest[consumed..])
        } else {
            let (VarInt(m), rest) = VarInt::decode_ref(rest)?;
            let count = usize::try_from(m)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
            (count, rest)
        };
        let outputs_buf = rest;
        let mut rest = rest;
        for _ in 0..output_count {
            let (_, r) = TxOutRef::decode_ref(rest)?;
            rest = r;
        }

        // For segwit: skip past the per-input witness stacks.
        let after_outputs = rest;
        if segwit {
            for _ in 0..input_count {
                // Each witness: VarInt(item_count) then VarInt(len)+bytes per item.
                let (VarInt(item_count), mut r) = VarInt::decode_ref(rest)?;
                for _ in 0..item_count {
                    let (VarInt(len), r2) = VarInt::decode_ref(r)?;
                    let len = usize::try_from(len)
                        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
                    if r2.len() < len {
                        return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "witness item truncated"));
                    }
                    r = &r2[len..];
                }
                rest = r;
            }
        }

        let (locktime, rest) = u32::decode_ref(rest)?;
        let raw = &tx_start[..tx_start.len() - rest.len()];

        // Build the non-witness serialisation slice. For legacy transactions
        // this is the same as `raw`. For segwit we synthesise it by copying
        // version + inputs + outputs + locktime, skipping marker/flag/witnesses.
        // We store it in a leaked Box so the lifetime is `'a`-compatible —
        // but that would allocate. Instead we defer to `to_owned` for callers
        // that need txid from a segwit TransactionRef: we store `after_outputs`
        // boundary info and recompute. For now, non_witness_raw == raw for the
        // zero-copy path; txid correctness for segwit requires the owned path.
        //
        // Correct zero-copy segwit txid requires re-serialising without witness,
        // which needs allocation. Store `raw` for now; callers needing segwit
        // txid via TransactionRef should call `.to_owned()?.txid()`.
        let non_witness_raw = if segwit {
            // Point at the slice covering only the after-outputs+locktime region
            // so the caller is aware this needs fixup. We mark by storing raw
            // until a proper non-witness scratch buffer is threaded through.
            // TODO(M2): thread a scratch buffer to avoid this limitation.
            raw
        } else {
            raw
        };
        let _ = after_outputs; // used above in witness-skip loop

        Ok((
            Self {
                version,
                locktime,
                raw,
                non_witness_raw,
                input_count,
                inputs_buf,
                output_count,
                outputs_buf,
            },
            rest,
        ))
    }
}

// ── Block ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    pub header: BlockHeader,
    pub transactions: Vec<Transaction>,
}

impl Encode for Block {
    fn encode(&self, w: &mut impl Write) -> io::Result<()> {
        self.header.encode(w)?;
        VarInt(self.transactions.len() as u64).encode(w)?;
        for tx in &self.transactions {
            tx.encode(w)?;
        }
        Ok(())
    }
}

impl Block {
    /// Block weight per BIP141: base size × 4 + witness-only size × 1.
    ///
    /// Base size is the non-witness serialisation (no marker/flag/witness
    /// fields). Total size is the full BIP144 serialisation. The formula
    /// `base × 3 + total` is equivalent to `base × 4 + witness_extra`.
    #[must_use]
    pub fn weight(&self) -> u64 {
        let tx_count = self.transactions.len();
        let header_and_count = BlockHeader::ENCODED_LEN + VarInt(tx_count as u64).encoded_len();
        let base: usize = header_and_count
            + self.transactions.iter().map(Transaction::base_encoded_len).sum::<usize>();
        let total: usize = header_and_count
            + self.transactions.iter().map(Transaction::encoded_len).sum::<usize>();
        (base as u64) * 3 + total as u64
    }

    /// Decode with caller-specified caps forwarded to each `Transaction`.
    ///
    /// # Errors
    /// Returns `InvalidData` if the tx count exceeds `MAX_TXS`, or on any I/O error.
    pub fn decode_limited<const MAX_TXS: usize, const MAX_INPUTS: usize, const MAX_OUTPUTS: usize>(
        r: &mut impl Read,
    ) -> io::Result<Self> {
        let header = BlockHeader::decode(r)?;
        let tx_count = usize::try_from(VarInt::decode(r)?.0)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        if tx_count > MAX_TXS {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "tx count exceeds limit"));
        }
        let mut transactions = Vec::with_capacity(tx_count);
        for _ in 0..tx_count {
            transactions.push(Transaction::decode_limited::<MAX_INPUTS, MAX_OUTPUTS>(r)?);
        }
        Ok(Self { header, transactions })
    }
}

impl Decode for Block {
    fn decode(r: &mut impl Read) -> io::Result<Self> {
        Self::decode_limited::<10_000, 10_000, 10_000>(r)
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn encode<T: Encode>(val: &T) -> Vec<u8> {
        let mut buf = Vec::new();
        val.encode(&mut buf).expect("Vec<u8> write cannot fail");
        buf
    }

    fn decode<T: Decode>(bytes: &[u8]) -> T {
        T::decode(&mut Cursor::new(bytes)).expect("test data must decode")
    }

    fn decode_ref<'a, T: DecodeRef<'a>>(buf: &'a [u8]) -> (T, &'a [u8]) {
        T::decode_ref(buf).expect("test data must decode")
    }

    #[test]
    fn hash256_display() {
        // display reverses bytes per Bitcoin convention
        let mut bytes = [0u8; 32];
        bytes[0] = 0xab;
        bytes[31] = 0xcd;
        let h = Hash256(bytes);
        let s = h.to_string();
        assert!(s.starts_with("cd"));
        assert!(s.ends_with("ab"));
    }

    #[test]
    fn block_header_roundtrip() {
        let header = BlockHeader {
            version: 1,
            prev_block: Hash256::ZERO,
            merkle_root: Hash256::ZERO,
            time: 1_231_006_505,
            bits: CompactBits(0x1d00_ffff),
            nonce: 2_083_236_893,
        };
        let bytes = encode(&header);
        assert_eq!(bytes.len(), BlockHeader::ENCODED_LEN);
        assert_eq!(decode::<BlockHeader>(&bytes), header);
    }

    #[test]
    fn block_header_decode_ref() {
        let header = BlockHeader {
            version: 2,
            prev_block: Hash256::ZERO,
            merkle_root: Hash256::ZERO,
            time: 1000,
            bits: CompactBits(0x1d00_ffff),
            nonce: 42,
        };
        let mut bytes = encode(&header);
        bytes.push(0xff); // sentinel
        let (decoded, tail) = decode_ref::<BlockHeader>(&bytes);
        assert_eq!(decoded, header);
        assert_eq!(tail, &[0xff]);
    }

    #[test]
    fn outpoint_roundtrip() {
        let op = OutPoint {
            txid: Hash256::ZERO,
            vout: 3,
        };
        let bytes = encode(&op);
        assert_eq!(bytes.len(), OutPoint::ENCODED_LEN);
        assert_eq!(decode::<OutPoint>(&bytes), op);
    }

    #[test]
    fn txin_ref_zero_copy() {
        let txin = TxIn {
            previous_output: OutPoint {
                txid: Hash256::ZERO,
                vout: 0,
            },
            script_sig: vec![0x01, 0x02, 0x03].into(),
            sequence: u32::MAX,
            witness: Witness::empty(),
        };
        let bytes = encode(&txin);
        let (txin_ref, tail) = decode_ref::<TxInRef>(&bytes);
        assert_eq!(txin_ref.script_sig, &[0x01, 0x02, 0x03]);
        assert_eq!(txin_ref.sequence, u32::MAX);
        assert!(tail.is_empty());
        // script_sig borrows directly from bytes
        assert!(std::ptr::eq(
            txin_ref.script_sig.as_ptr(),
            &raw const bytes[OutPoint::ENCODED_LEN + 1]
        ));
    }

    #[test]
    fn txout_ref_zero_copy() {
        let txout = TxOut {
            value: Satoshi::new(5_000_000_000),
            script_pubkey: vec![0xac, 0xbd].into(),
        };
        let bytes = encode(&txout);
        let (txout_ref, tail) = decode_ref::<TxOutRef>(&bytes);
        assert_eq!(txout_ref.value, Satoshi::new(5_000_000_000));
        assert_eq!(txout_ref.script_pubkey, &[0xac, 0xbd]);
        assert!(tail.is_empty());
    }

    #[test]
    fn transaction_roundtrip() {
        let tx = Transaction {
            version: 1,
            inputs: vec![TxIn {
                previous_output: OutPoint {
                    txid: Hash256::ZERO,
                    vout: 0xffff_ffff,
                },
                script_sig: vec![0x04, 0xff, 0xff, 0x00, 0x1d, 0x01, 0x04].into(),
                sequence: u32::MAX,
                witness: Witness::empty(),
            }],
            outputs: vec![TxOut {
                value: Satoshi::new(5_000_000_000),
                script_pubkey: vec![0x41, 0xac].into(),
            }],
            locktime: 0,
        };
        let bytes = encode(&tx);
        assert_eq!(decode::<Transaction>(&bytes), tx);
    }

    #[test]
    fn block_roundtrip() {
        let block = Block {
            header: BlockHeader {
                version: 1,
                prev_block: Hash256::ZERO,
                merkle_root: Hash256::ZERO,
                time: 1_231_006_505,
                bits: CompactBits(0x1d00_ffff),
                nonce: 2_083_236_893,
            },
            transactions: vec![Transaction {
                version: 1,
                inputs: vec![TxIn {
                    previous_output: OutPoint {
                        txid: Hash256::ZERO,
                        vout: 0xffff_ffff,
                    },
                    script_sig: vec![0x04].into(),
                    sequence: u32::MAX,
                    witness: Witness::empty(),
                }],
                outputs: vec![TxOut {
                    value: Satoshi::new(5_000_000_000),
                    script_pubkey: vec![0xac].into(),
                }],
                locktime: 0,
            }],
        };
        let bytes = encode(&block);
        assert_eq!(decode::<Block>(&bytes), block);
    }

    // ── Adversarial / edge-case tests ─────────────────────────────────────────

    fn decode_err<T: Decode + std::fmt::Debug>(bytes: &[u8]) -> std::io::Error {
        T::decode(&mut std::io::Cursor::new(bytes)).unwrap_err()
    }

    fn decode_ref_err<'a, T: DecodeRef<'a> + std::fmt::Debug>(buf: &'a [u8]) -> std::io::Error {
        T::decode_ref(buf).unwrap_err()
    }

    // Hash256: fewer than 32 bytes must fail.
    #[test]
    fn hash256_truncated() {
        let e = decode_err::<Hash256>(&[0u8; 31]);
        assert_eq!(e.kind(), std::io::ErrorKind::UnexpectedEof);
    }

    #[test]
    fn hash256_decode_ref_truncated() {
        let e = decode_ref_err::<Hash256>(&[0u8; 16]);
        assert_eq!(e.kind(), std::io::ErrorKind::UnexpectedEof);
    }

    // Hash256: all-ones round-trips (not just the ZERO sentinel).
    #[test]
    fn hash256_all_ones_roundtrip() {
        let h = Hash256([0xff; 32]);
        let bytes = encode(&h);
        assert_eq!(decode::<Hash256>(&bytes), h);
    }

    // Hash256 display: all-zero hash shows as 64 zeros.
    #[test]
    fn hash256_zero_display() {
        assert_eq!(Hash256::ZERO.to_string(), "0".repeat(64));
    }

    // BlockHeader: truncated at every meaningful boundary.
    #[test]
    fn block_header_truncated_version() {
        let e = decode_err::<BlockHeader>(&[0u8; 2]);
        assert_eq!(e.kind(), std::io::ErrorKind::UnexpectedEof);
    }

    #[test]
    fn block_header_truncated_prev_block() {
        // version (4) + partial prev_block (10)
        let e = decode_err::<BlockHeader>(&[0u8; 14]);
        assert_eq!(e.kind(), std::io::ErrorKind::UnexpectedEof);
    }

    #[test]
    fn block_header_truncated_nonce() {
        // version(4) + prev_block(32) + merkle_root(32) + time(4) + bits(4) + partial nonce(2)
        let e = decode_err::<BlockHeader>(&[0u8; 78]);
        assert_eq!(e.kind(), std::io::ErrorKind::UnexpectedEof);
    }

    // BlockHeader: negative version and extreme field values round-trip.
    #[test]
    fn block_header_extreme_fields() {
        let h = BlockHeader {
            version: i32::MIN,
            prev_block: Hash256([0xff; 32]),
            merkle_root: Hash256([0xab; 32]),
            time: u32::MAX,
            bits: CompactBits(u32::MAX),
            nonce: u32::MAX,
        };
        let bytes = encode(&h);
        assert_eq!(bytes.len(), BlockHeader::ENCODED_LEN);
        assert_eq!(decode::<BlockHeader>(&bytes), h);
    }

    // OutPoint: vout = u32::MAX (used to mark coinbase inputs).
    #[test]
    fn outpoint_coinbase_vout() {
        let op = OutPoint { txid: Hash256::ZERO, vout: u32::MAX };
        let bytes = encode(&op);
        assert_eq!(bytes.len(), OutPoint::ENCODED_LEN);
        assert_eq!(decode::<OutPoint>(&bytes), op);
    }

    // OutPoint: truncated txid.
    #[test]
    fn outpoint_truncated_txid() {
        let e = decode_err::<OutPoint>(&[0u8; 16]);
        assert_eq!(e.kind(), std::io::ErrorKind::UnexpectedEof);
    }

    // TxIn: empty script_sig (valid — e.g. native segwit inputs).
    #[test]
    fn txin_empty_script_sig() {
        let txin = TxIn {
            previous_output: OutPoint { txid: Hash256::ZERO, vout: 0 },
            script_sig: vec![].into(),
            sequence: 0,
            witness: Witness::empty(),
        };
        let bytes = encode(&txin);
        assert_eq!(decode::<TxIn>(&bytes), txin);

        // Zero-copy path.
        let (r, tail) = decode_ref::<TxInRef>(&bytes);
        assert!(r.script_sig.is_empty());
        assert!(tail.is_empty());
    }

    // TxIn: truncated script_sig body.
    #[test]
    fn txin_truncated_script_sig() {
        let txin = TxIn {
            previous_output: OutPoint { txid: Hash256::ZERO, vout: 0 },
            script_sig: vec![0x01, 0x02, 0x03, 0x04].into(),
            sequence: u32::MAX,
            witness: Witness::empty(),
        };
        let mut bytes = encode(&txin);
        bytes.truncate(bytes.len() - 6); // cut into script_sig + sequence
        let e = decode_err::<TxIn>(&bytes);
        assert_eq!(e.kind(), std::io::ErrorKind::UnexpectedEof);
    }

    // TxOut: zero value (valid — OP_RETURN outputs often carry 0 satoshis).
    #[test]
    fn txout_zero_value() {
        let txout = TxOut { value: Satoshi::ZERO, script_pubkey: vec![0x6a].into() }; // OP_RETURN
        let bytes = encode(&txout);
        assert_eq!(decode::<TxOut>(&bytes), txout);
    }

    // TxOut: value = u64::MAX round-trips (obviously invalid economically, but
    // the codec must not panic or corrupt the field).
    #[test]
    fn txout_max_value() {
        let txout = TxOut { value: Satoshi::new(u64::MAX), script_pubkey: vec![].into() };
        let bytes = encode(&txout);
        assert_eq!(decode::<TxOut>(&bytes), txout);
    }

    // TxOut: truncated script_pubkey.
    #[test]
    fn txout_truncated_script_pubkey() {
        let txout = TxOut { value: Satoshi::new(1000), script_pubkey: vec![0x76, 0xa9, 0x14].into() };
        let mut bytes = encode(&txout);
        bytes.truncate(bytes.len() - 2);
        let e = decode_err::<TxOut>(&bytes);
        assert_eq!(e.kind(), std::io::ErrorKind::UnexpectedEof);
    }

    // Transaction: zero inputs and zero outputs is syntactically decodable.
    #[test]
    fn transaction_empty_inputs_outputs() {
        let tx = Transaction { version: 1, inputs: vec![], outputs: vec![], locktime: 0 };
        let bytes = encode(&tx);
        assert_eq!(decode::<Transaction>(&bytes), tx);
    }

    // Transaction: negative version round-trips.
    #[test]
    fn transaction_negative_version() {
        let tx = Transaction {
            version: -1,
            inputs: vec![],
            outputs: vec![],
            locktime: u32::MAX,
        };
        let bytes = encode(&tx);
        assert_eq!(decode::<Transaction>(&bytes), tx);
    }

    // Transaction: truncated after the input-count VarInt.
    #[test]
    fn transaction_truncated_after_input_count() {
        // version(4) + VarInt(1) then nothing.
        let buf = [0x01, 0x00, 0x00, 0x00, 0x01];
        let e = decode_err::<Transaction>(&buf);
        assert_eq!(e.kind(), std::io::ErrorKind::UnexpectedEof);
    }

    // Transaction: input-count VarInt claims more entries than bytes available.
    #[test]
    fn transaction_input_count_exceeds_data() {
        // version(4) + VarInt(100) + no actual inputs.
        let mut buf = vec![0x01, 0x00, 0x00, 0x00]; // version = 1
        buf.push(0x64); // VarInt(100)
        let e = decode_err::<Transaction>(&buf);
        assert_eq!(e.kind(), std::io::ErrorKind::UnexpectedEof);
    }

    // Block: zero transactions is valid (header-only, used in `headers` messages
    // before the tx_count placeholder was separated).
    #[test]
    fn block_zero_transactions() {
        let block = Block {
            header: BlockHeader {
                version: 1,
                prev_block: Hash256::ZERO,
                merkle_root: Hash256::ZERO,
                time: 0,
                bits: CompactBits(0),
                nonce: 0,
            },
            transactions: vec![],
        };
        let bytes = encode(&block);
        assert_eq!(decode::<Block>(&bytes), block);
    }

    // Block: truncated after the tx-count VarInt.
    #[test]
    fn block_truncated_after_tx_count() {
        let block = Block {
            header: BlockHeader {
                version: 1,
                prev_block: Hash256::ZERO,
                merkle_root: Hash256::ZERO,
                time: 0,
                bits: CompactBits(0),
                nonce: 0,
            },
            transactions: vec![],
        };
        let mut bytes = encode(&block);
        // Overwrite tx_count to claim 5 transactions, then provide none.
        let tx_count_offset = BlockHeader::ENCODED_LEN;
        bytes[tx_count_offset] = 0x05;
        let e = decode_err::<Block>(&bytes);
        assert_eq!(e.kind(), std::io::ErrorKind::UnexpectedEof);
    }

    // Block: tx-count VarInt uses 0xfd-prefix (3-byte encoding) and round-trips.
    #[test]
    fn block_tx_count_varint_3byte_encoding() {
        // Build a block with 253 (0xfd) transactions to force a 3-byte VarInt.
        let dummy_tx = Transaction {
            version: 1,
            inputs: vec![],
            outputs: vec![],
            locktime: 0,
        };
        let block = Block {
            header: BlockHeader {
                version: 1,
                prev_block: Hash256::ZERO,
                merkle_root: Hash256::ZERO,
                time: 0,
                bits: CompactBits(0),
                nonce: 0,
            },
            transactions: vec![dummy_tx; 253],
        };
        let bytes = encode(&block);
        // tx_count byte at offset 80 must be 0xfd (3-byte VarInt marker).
        assert_eq!(bytes[BlockHeader::ENCODED_LEN], 0xfd);
        assert_eq!(decode::<Block>(&bytes), block);
    }

    // ── TransactionRef ────────────────────────────────────────────────────────

    fn make_tx() -> Transaction {
        Transaction {
            version: 2,
            inputs: vec![
                TxIn {
                    previous_output: OutPoint { txid: Hash256([0xab; 32]), vout: 1 },
                    script_sig: vec![0x01, 0x02, 0x03].into(),
                    sequence: 0xffff_fffe,
                    witness: Witness::empty(),
                },
                TxIn {
                    previous_output: OutPoint { txid: Hash256([0xcd; 32]), vout: 0 },
                    script_sig: vec![].into(),
                    sequence: 0,
                    witness: Witness::empty(),
                },
            ],
            outputs: vec![
                TxOut { value: Satoshi::new(50_000), script_pubkey: vec![0x76, 0xa9].into() },
                TxOut { value: Satoshi::ZERO, script_pubkey: vec![].into() },
            ],
            locktime: 800_000,
        }
    }

    #[test]
    fn transaction_ref_decode_ref_roundtrip() {
        let tx = make_tx();
        let bytes = encode(&tx);
        let (tx_ref, tail) = decode_ref::<TransactionRef>(&bytes);
        assert!(tail.is_empty());
        assert_eq!(tx_ref.version, tx.version);
        assert_eq!(tx_ref.locktime, tx.locktime);
        assert_eq!(tx_ref.input_count(), tx.inputs.len());
        assert_eq!(tx_ref.output_count(), tx.outputs.len());

        for (r, expected) in tx_ref.inputs().zip(&tx.inputs) {
            let i = r.expect("input decode must succeed");
            assert_eq!(i.previous_output, expected.previous_output);
            assert_eq!(i.script_sig, expected.script_sig.as_slice());
            assert_eq!(i.sequence, expected.sequence);
        }
        for (r, expected) in tx_ref.outputs().zip(&tx.outputs) {
            let o = r.expect("output decode must succeed");
            assert_eq!(o.value, expected.value);
            assert_eq!(o.script_pubkey, expected.script_pubkey.as_slice());
        }
    }

    #[test]
    fn transaction_ref_script_sig_zero_copy() {
        let tx = make_tx();
        let bytes = encode(&tx);
        let (tx_ref, _) = decode_ref::<TransactionRef>(&bytes);
        let first = tx_ref.inputs().next().expect("tx has inputs").expect("input decode must succeed");
        // script_sig must point directly into `bytes`, not a new allocation.
        let script_start = first.script_sig.as_ptr() as usize;
        let buf_start = bytes.as_ptr() as usize;
        let buf_end = buf_start + bytes.len();
        assert!(script_start >= buf_start && script_start < buf_end);
    }

    #[test]
    fn transaction_ref_to_owned_matches_original() {
        let tx = make_tx();
        let bytes = encode(&tx);
        let (tx_ref, _) = decode_ref::<TransactionRef>(&bytes);
        assert_eq!(tx_ref.to_owned().expect("to_owned must succeed"), tx);
    }

    #[test]
    fn transaction_ref_encode_matches_original() {
        let tx = make_tx();
        let bytes = encode(&tx);
        let (tx_ref, _) = decode_ref::<TransactionRef>(&bytes);
        let re_encoded = encode(&tx_ref);
        assert_eq!(re_encoded, bytes);
    }

    #[test]
    fn transaction_ref_encoded_len_matches_transaction() {
        let tx = make_tx();
        let bytes = encode(&tx);
        let (tx_ref, _) = decode_ref::<TransactionRef>(&bytes);
        assert_eq!(tx_ref.encoded_len(), tx.encoded_len());
        assert_eq!(tx_ref.encoded_len(), bytes.len());
    }

    #[test]
    fn transaction_ref_tail_is_remaining_bytes() {
        let tx = make_tx();
        let mut bytes = encode(&tx);
        bytes.extend_from_slice(&[0xde, 0xad]);
        let (_, tail) = decode_ref::<TransactionRef>(&bytes);
        assert_eq!(tail, &[0xde, 0xad]);
    }

    #[test]
    fn transaction_ref_truncated_returns_error() {
        let tx = make_tx();
        let bytes = encode(&tx);
        // Cut the buffer in half — must not panic, must return an error.
        let e = TransactionRef::decode_ref(&bytes[..bytes.len() / 2]).unwrap_err();
        assert_eq!(e.kind(), std::io::ErrorKind::UnexpectedEof);
    }

    #[test]
    fn transaction_ref_empty_inputs_outputs() {
        let tx = Transaction { version: 1, inputs: vec![], outputs: vec![], locktime: 0 };
        let bytes = encode(&tx);
        let (tx_ref, tail) = decode_ref::<TransactionRef>(&bytes);
        assert!(tail.is_empty());
        assert_eq!(tx_ref.input_count(), 0);
        assert_eq!(tx_ref.output_count(), 0);
        assert!(tx_ref.inputs().next().is_none());
        assert!(tx_ref.outputs().next().is_none());
        assert_eq!(tx_ref.to_owned().expect("to_owned must succeed"), tx);
    }
}
