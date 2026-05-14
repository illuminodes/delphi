use std::io::{Read, Write};

use crate::codec::{Decode, DecodeRef, Encode, Error};
use crate::difficulty::CompactBits;
use crate::hash::{ByteSliceExt, Hash256};

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
    #[allow(clippy::missing_panics_doc)]
    pub fn to_bytes(&self) -> [u8; Self::ENCODED_LEN] {
        let mut buf = [0u8; Self::ENCODED_LEN];
        self.encode(&mut buf.as_mut_slice())
            .expect("fixed-size write cannot fail");
        buf
    }

    /// `SHA256d` of the 80-byte serialised header — the canonical block hash.
    #[must_use]
    pub fn hash(&self) -> Hash256 {
        self.to_bytes().sha256d()
    }

    /// Deserialize from a fixed 80-byte slice.
    ///
    /// # Errors
    /// Returns an error if the bytes are malformed (in practice, never for a
    /// fixed-size 80-byte input).
    pub fn from_bytes(b: &[u8; Self::ENCODED_LEN]) -> Result<Self, Error> {
        let mut r = b.as_slice();
        let version = i32::decode(&mut r)?;
        let prev_block = Hash256::decode(&mut r)?;
        let merkle_root = Hash256::decode(&mut r)?;
        let time = u32::decode(&mut r)?;
        let bits = CompactBits::decode(&mut r)?;
        let nonce = u32::decode(&mut r)?;
        Ok(Self {
            version,
            prev_block,
            merkle_root,
            time,
            bits,
            nonce,
        })
    }
}

impl Encode for BlockHeader {
    #[inline]
    fn encode(&self, w: &mut impl Write) -> Result<(), Error> {
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
    fn decode(r: &mut impl Read) -> Result<Self, Error> {
        let mut buf = [0u8; Self::ENCODED_LEN];
        r.read_exact(&mut buf)?;
        Self::from_bytes(&buf)
    }
}

impl<'a> DecodeRef<'a> for BlockHeader {
    #[inline]
    fn decode_ref(buf: &'a [u8]) -> Result<(Self, &'a [u8]), Error> {
        if buf.len() < Self::ENCODED_LEN {
            return Err(Error::UnexpectedEof);
        }
        let (head, tail) = buf.split_at(Self::ENCODED_LEN);
        Ok((
            Self::from_bytes(head.try_into().map_err(|_| Error::UnexpectedEof)?)?,
            tail,
        ))
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
        let header = BlockHeader {
            version: 1,
            prev_block: Hash256::ZERO,
            merkle_root: Hash256::ZERO,
            time: 1_231_006_505,
            bits: CompactBits::new(0x1d00_ffff),
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
            bits: CompactBits::new(0x1d00_ffff),
            nonce: 42,
        };
        let mut bytes = encode(&header);
        bytes.push(0xff); // sentinel
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
        // version (4) + partial prev_block (10)
        let e = decode_err::<BlockHeader>(&[0u8; 14]);
        assert!(matches!(e, Error::UnexpectedEof | Error::Io(_)));
    }

    #[test]
    fn block_header_truncated_nonce() {
        // version(4) + prev_block(32) + merkle_root(32) + time(4) + bits(4) + partial nonce(2)
        let e = decode_err::<BlockHeader>(&[0u8; 78]);
        assert!(matches!(e, Error::UnexpectedEof | Error::Io(_)));
    }

    #[test]
    fn block_header_extreme_fields() {
        let h = BlockHeader {
            version: i32::MIN,
            prev_block: Hash256::from_bytes([0xff; 32]),
            merkle_root: Hash256::from_bytes([0xab; 32]),
            time: u32::MAX,
            bits: CompactBits::new(u32::MAX),
            nonce: u32::MAX,
        };
        let bytes = encode(&h);
        assert_eq!(bytes.len(), BlockHeader::ENCODED_LEN);
        assert_eq!(decode::<BlockHeader>(&bytes), h);
    }
}
