use std::io::{Read, Write};

use crate::codec::{Decode, DecodeRef, Encode, Error};
use crate::difficulty::{BlockVersion, CompactBits, Nonce};
use crate::hash::Hash256;

/// 80-byte fixed-size block header.
///
/// `#[repr(C)]` guarantees the field layout matches the Bitcoin wire format
/// exactly — no padding, no reordering. `size_of::<BlockHeader>() == 80`.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct BlockHeader {
    version: BlockVersion,
    prev_block: Hash256,
    merkle_root: Hash256,
    time: u32,
    bits: CompactBits,
    nonce: Nonce,
}

const _: () = assert!(std::mem::size_of::<BlockHeader>() == 80);

impl BlockHeader {
    pub const ENCODED_LEN: usize = 80;

    #[must_use]
    pub fn new(
        version: i32,
        prev_block: Hash256,
        merkle_root: Hash256,
        time: u32,
        bits: CompactBits,
        nonce: u32,
    ) -> Self {
        Self {
            version: BlockVersion::new(version),
            prev_block,
            merkle_root,
            time,
            bits,
            nonce: Nonce::new(nonce),
        }
    }

    /// The raw 80-byte wire representation.
    #[inline]
    #[must_use]
    pub fn to_bytes(&self) -> [u8; Self::ENCODED_LEN] {
        // SAFETY: #[repr(C)] guarantees layout matches wire format.
        // All fields are little-endian primitives or [u8] arrays.
        unsafe { std::ptr::read(std::ptr::from_ref(self).cast()) }
    }

    /// The raw 80-byte wire representation as a slice.
    #[inline]
    #[must_use]
    pub fn as_bytes(&self) -> &[u8; Self::ENCODED_LEN] {
        unsafe { &*std::ptr::from_ref(self).cast() }
    }

    /// Construct from raw 80-byte wire representation.
    #[must_use]
    pub fn from_bytes(b: [u8; Self::ENCODED_LEN]) -> Self {
        unsafe { std::ptr::read(b.as_ptr().cast()) }
    }

    /// Construct from a byte slice reference.
    #[must_use]
    pub fn from_slice(b: &[u8; Self::ENCODED_LEN]) -> Self {
        Self::from_bytes(*b)
    }

    /// `SHA256d` of the 80-byte header — the canonical block hash.
    #[inline]
    #[must_use]
    pub fn hash(&self) -> Hash256 {
        Hash256::hash(self.as_bytes())
    }

    #[inline]
    #[must_use]
    pub fn version(&self) -> i32 {
        self.version.to_i32()
    }

    #[inline]
    #[must_use]
    pub fn prev_block(&self) -> Hash256 {
        self.prev_block
    }

    #[inline]
    #[must_use]
    pub fn merkle_root(&self) -> Hash256 {
        self.merkle_root
    }

    #[inline]
    #[must_use]
    pub fn time(&self) -> u32 {
        self.time
    }

    #[inline]
    #[must_use]
    pub fn bits(&self) -> CompactBits {
        self.bits
    }

    #[inline]
    #[must_use]
    pub fn nonce(&self) -> u32 {
        self.nonce.to_u32()
    }

    #[inline]
    pub fn set_version(&mut self, v: i32) {
        self.version = BlockVersion::new(v);
    }

    #[inline]
    pub fn set_prev_block(&mut self, h: Hash256) {
        self.prev_block = h;
    }

    #[inline]
    pub fn set_merkle_root(&mut self, h: Hash256) {
        self.merkle_root = h;
    }

    #[inline]
    pub fn set_time(&mut self, t: u32) {
        self.time = t;
    }

    #[inline]
    pub fn set_bits(&mut self, b: CompactBits) {
        self.bits = b;
    }

    #[inline]
    pub fn set_nonce(&mut self, n: u32) {
        self.nonce = Nonce::new(n);
    }
}

impl std::fmt::Debug for BlockHeader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BlockHeader")
            .field("version", &self.version())
            .field("prev_block", &self.prev_block())
            .field("merkle_root", &self.merkle_root())
            .field("time", &self.time())
            .field("bits", &self.bits())
            .field("nonce", &self.nonce())
            .finish()
    }
}

impl Encode for BlockHeader {
    #[inline]
    fn encode(&self, w: &mut impl Write) -> Result<(), Error> {
        w.write_all(self.as_bytes()).map_err(Error::Io)
    }
}

impl Decode for BlockHeader {
    #[inline]
    fn decode(r: &mut impl Read) -> Result<Self, Error> {
        let mut buf = [0u8; Self::ENCODED_LEN];
        r.read_exact(&mut buf)?;
        Ok(Self::from_bytes(buf))
    }
}

impl<'a> DecodeRef<'a> for BlockHeader {
    #[inline]
    fn decode_ref(buf: &'a [u8]) -> Result<(Self, &'a [u8]), Error> {
        if buf.len() < Self::ENCODED_LEN {
            return Err(Error::UnexpectedEof);
        }
        let (head, tail) = buf.split_at(Self::ENCODED_LEN);
        let mut arr = [0u8; Self::ENCODED_LEN];
        arr.copy_from_slice(head);
        Ok((Self::from_bytes(arr), tail))
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use crate::codec::{Decode, DecodeRef, Encode, Error};
    use crate::difficulty::CompactBits;
    use crate::hash::Hash256;

    use super::BlockHeader;

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

    fn decode_err<T: Decode + std::fmt::Debug>(bytes: &[u8]) -> Error {
        T::decode(&mut std::io::Cursor::new(bytes)).unwrap_err()
    }

    #[test]
    fn block_header_roundtrip() {
        let header = BlockHeader::new(
            1,
            Hash256::ZERO,
            Hash256::ZERO,
            1_231_006_505,
            CompactBits::new(0x1d00_ffff),
            2_083_236_893,
        );
        let bytes = encode(&header);
        assert_eq!(bytes.len(), BlockHeader::ENCODED_LEN);
        assert_eq!(decode::<BlockHeader>(&bytes), header);
    }

    #[test]
    fn block_header_decode_ref() {
        let header = BlockHeader::new(
            2,
            Hash256::ZERO,
            Hash256::ZERO,
            1000,
            CompactBits::new(0x1d00_ffff),
            42,
        );
        let mut bytes = encode(&header);
        bytes.push(0xff);
        let (decoded, tail) = decode_ref::<BlockHeader>(&bytes);
        assert_eq!(decoded, header);
        assert_eq!(tail, &[0xff]);
    }

    #[test]
    fn block_header_truncated_version() {
        let e = decode_err::<BlockHeader>(&[0u8; 2]);
        assert!(matches!(e, Error::UnexpectedEof | Error::Io(_)));
    }

    #[test]
    fn block_header_truncated_prev_block() {
        let e = decode_err::<BlockHeader>(&[0u8; 14]);
        assert!(matches!(e, Error::UnexpectedEof | Error::Io(_)));
    }

    #[test]
    fn block_header_truncated_nonce() {
        let e = decode_err::<BlockHeader>(&[0u8; 78]);
        assert!(matches!(e, Error::UnexpectedEof | Error::Io(_)));
    }

    #[test]
    fn block_header_extreme_fields() {
        let h = BlockHeader::new(
            i32::MIN,
            Hash256::from_bytes([0xff; 32]),
            Hash256::from_bytes([0xab; 32]),
            u32::MAX,
            CompactBits::new(u32::MAX),
            u32::MAX,
        );
        let bytes = encode(&h);
        assert_eq!(bytes.len(), BlockHeader::ENCODED_LEN);
        assert_eq!(decode::<BlockHeader>(&bytes), h);
    }

    #[test]
    fn field_accessors_match_constructor() {
        let h = BlockHeader::new(
            0x0200_0000,
            Hash256::from_bytes([0xaa; 32]),
            Hash256::from_bytes([0xbb; 32]),
            1_700_000_000,
            CompactBits::new(0x1d00_ffff),
            42,
        );
        assert_eq!(h.version(), 0x0200_0000);
        assert_eq!(h.prev_block(), Hash256::from_bytes([0xaa; 32]));
        assert_eq!(h.merkle_root(), Hash256::from_bytes([0xbb; 32]));
        assert_eq!(h.time(), 1_700_000_000);
        assert_eq!(h.bits(), CompactBits::new(0x1d00_ffff));
        assert_eq!(h.nonce(), 42);
    }

    #[test]
    fn set_then_get_roundtrip() {
        let mut h = BlockHeader::new(1, Hash256::ZERO, Hash256::ZERO, 0, CompactBits::new(0), 0);
        h.set_version(3);
        h.set_nonce(999);
        h.set_bits(CompactBits::new(0x1234_5678));
        assert_eq!(h.version(), 3);
        assert_eq!(h.nonce(), 999);
        assert_eq!(h.bits(), CompactBits::new(0x1234_5678));
    }

    #[test]
    fn size_is_80() {
        assert_eq!(std::mem::size_of::<BlockHeader>(), 80);
    }
}
