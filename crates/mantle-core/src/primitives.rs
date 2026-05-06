use std::fmt;
use std::io::{self, Read, Write};

use crate::codec::{Decode, DecodeRef, Encode, VarInt};

// ── Hash256 ───────────────────────────────────────────────────────────────────

/// A 32-byte hash (txid, block hash, merkle root, etc.). Stored as raw bytes,
/// displayed in reversed byte order per Bitcoin convention.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Hash256(pub [u8; 32]);

impl Hash256 {
    pub const ZERO: Self = Self([0u8; 32]);
}

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
        Ok((Self(head.try_into().unwrap()), tail))
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
    pub bits: u32,
    pub nonce: u32,
}

impl BlockHeader {
    pub const ENCODED_LEN: usize = 80;
}

impl Encode for BlockHeader {
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
    fn decode(r: &mut impl Read) -> io::Result<Self> {
        Ok(Self {
            version: i32::decode(r)?,
            prev_block: Hash256::decode(r)?,
            merkle_root: Hash256::decode(r)?,
            time: u32::decode(r)?,
            bits: u32::decode(r)?,
            nonce: u32::decode(r)?,
        })
    }
}

impl<'a> DecodeRef<'a> for BlockHeader {
    fn decode_ref(buf: &'a [u8]) -> io::Result<(Self, &'a [u8])> {
        let (version, buf) = i32::decode_ref(buf)?;
        let (prev_block, buf) = Hash256::decode_ref(buf)?;
        let (merkle_root, buf) = Hash256::decode_ref(buf)?;
        let (time, buf) = u32::decode_ref(buf)?;
        let (bits, buf) = u32::decode_ref(buf)?;
        let (nonce, buf) = u32::decode_ref(buf)?;
        Ok((
            Self {
                version,
                prev_block,
                merkle_root,
                time,
                bits,
                nonce,
            },
            buf,
        ))
    }
}

// ── OutPoint ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

// ── TxIn ──────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TxIn {
    pub previous_output: OutPoint,
    /// Serialized script or witness program.
    pub script_sig: Vec<u8>,
    pub sequence: u32,
}

impl Encode for TxIn {
    fn encode(&self, w: &mut impl Write) -> io::Result<()> {
        self.previous_output.encode(w)?;
        self.script_sig.encode(w)?;
        self.sequence.encode(w)
    }
}

impl Decode for TxIn {
    fn decode(r: &mut impl Read) -> io::Result<Self> {
        Ok(Self {
            previous_output: OutPoint::decode(r)?,
            script_sig: Vec::<u8>::decode(r)?,
            sequence: u32::decode(r)?,
        })
    }
}

/// Borrowed view of a `TxIn` — script_sig is a slice into the source buffer.
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
    /// Value in satoshis.
    pub value: u64,
    pub script_pubkey: Vec<u8>,
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
            value: u64::decode(r)?,
            script_pubkey: Vec::<u8>::decode(r)?,
        })
    }
}

/// Borrowed view of a `TxOut` — script_pubkey is a slice into the source buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TxOutRef<'a> {
    pub value: u64,
    pub script_pubkey: &'a [u8],
}

impl<'a> DecodeRef<'a> for TxOutRef<'a> {
    fn decode_ref(buf: &'a [u8]) -> io::Result<(Self, &'a [u8])> {
        let (value, buf) = u64::decode_ref(buf)?;
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

// ── Transaction ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transaction {
    pub version: i32,
    pub inputs: Vec<TxIn>,
    pub outputs: Vec<TxOut>,
    pub locktime: u32,
}

impl Encode for Transaction {
    fn encode(&self, w: &mut impl Write) -> io::Result<()> {
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

impl Decode for Transaction {
    fn decode(r: &mut impl Read) -> io::Result<Self> {
        let version = i32::decode(r)?;
        let input_count = VarInt::decode(r)?.0 as usize;
        let mut inputs = Vec::with_capacity(input_count);
        for _ in 0..input_count {
            inputs.push(TxIn::decode(r)?);
        }
        let output_count = VarInt::decode(r)?.0 as usize;
        let mut outputs = Vec::with_capacity(output_count);
        for _ in 0..output_count {
            outputs.push(TxOut::decode(r)?);
        }
        let locktime = u32::decode(r)?;
        Ok(Self {
            version,
            inputs,
            outputs,
            locktime,
        })
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

impl Decode for Block {
    fn decode(r: &mut impl Read) -> io::Result<Self> {
        let header = BlockHeader::decode(r)?;
        let tx_count = VarInt::decode(r)?.0 as usize;
        let mut transactions = Vec::with_capacity(tx_count);
        for _ in 0..tx_count {
            transactions.push(Transaction::decode(r)?);
        }
        Ok(Self {
            header,
            transactions,
        })
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn encode<T: Encode>(val: &T) -> Vec<u8> {
        let mut buf = Vec::new();
        val.encode(&mut buf).unwrap();
        buf
    }

    fn decode<T: Decode>(bytes: &[u8]) -> T {
        T::decode(&mut Cursor::new(bytes)).unwrap()
    }

    fn decode_ref<'a, T: DecodeRef<'a>>(buf: &'a [u8]) -> (T, &'a [u8]) {
        T::decode_ref(buf).unwrap()
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
            time: 1231006505,
            bits: 0x1d00ffff,
            nonce: 2083236893,
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
            bits: 0x1d00ffff,
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
            script_sig: vec![0x01, 0x02, 0x03],
            sequence: u32::MAX,
        };
        let bytes = encode(&txin);
        let (txin_ref, tail) = decode_ref::<TxInRef>(&bytes);
        assert_eq!(txin_ref.script_sig, &[0x01, 0x02, 0x03]);
        assert_eq!(txin_ref.sequence, u32::MAX);
        assert!(tail.is_empty());
        // script_sig borrows directly from bytes
        assert!(std::ptr::eq(
            txin_ref.script_sig.as_ptr(),
            &bytes[OutPoint::ENCODED_LEN + 1]
        ));
    }

    #[test]
    fn txout_ref_zero_copy() {
        let txout = TxOut {
            value: 5000000000,
            script_pubkey: vec![0xac, 0xbd],
        };
        let bytes = encode(&txout);
        let (txout_ref, tail) = decode_ref::<TxOutRef>(&bytes);
        assert_eq!(txout_ref.value, 5000000000);
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
                    vout: 0xffffffff,
                },
                script_sig: vec![0x04, 0xff, 0xff, 0x00, 0x1d, 0x01, 0x04],
                sequence: u32::MAX,
            }],
            outputs: vec![TxOut {
                value: 5000000000,
                script_pubkey: vec![0x41, 0xac],
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
                time: 1231006505,
                bits: 0x1d00ffff,
                nonce: 2083236893,
            },
            transactions: vec![Transaction {
                version: 1,
                inputs: vec![TxIn {
                    previous_output: OutPoint {
                        txid: Hash256::ZERO,
                        vout: 0xffffffff,
                    },
                    script_sig: vec![0x04],
                    sequence: u32::MAX,
                }],
                outputs: vec![TxOut {
                    value: 5000000000,
                    script_pubkey: vec![0xac],
                }],
                locktime: 0,
            }],
        };
        let bytes = encode(&block);
        assert_eq!(decode::<Block>(&bytes), block);
    }
}
