use std::fmt;
use std::io::{self, Read, Write};

use crate::codec::{Decode, DecodeRef, Encode, VarInt, decode_bytes_limited};

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
            script_sig: decode_bytes_limited::<10_000>(r)?,
            sequence: u32::decode(r)?,
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
            script_pubkey: decode_bytes_limited::<10_000>(r)?,
        })
    }
}

/// Borrowed view of a `TxOut` — `script_pubkey` is a slice into the source buffer.
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
        let input_count = usize::try_from(VarInt::decode(r)?.0)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        if input_count > MAX_INPUTS {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "input count exceeds limit"));
        }
        let mut inputs = Vec::with_capacity(input_count);
        for _ in 0..input_count {
            inputs.push(TxIn::decode(r)?);
        }
        let output_count = usize::try_from(VarInt::decode(r)?.0)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        if output_count > MAX_OUTPUTS {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "output count exceeds limit"));
        }
        let mut outputs = Vec::with_capacity(output_count);
        for _ in 0..output_count {
            outputs.push(TxOut::decode(r)?);
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
            bits: u32::MAX,
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
            script_sig: vec![],
            sequence: 0,
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
            script_sig: vec![0x01, 0x02, 0x03, 0x04],
            sequence: u32::MAX,
        };
        let mut bytes = encode(&txin);
        bytes.truncate(bytes.len() - 6); // cut into script_sig + sequence
        let e = decode_err::<TxIn>(&bytes);
        assert_eq!(e.kind(), std::io::ErrorKind::UnexpectedEof);
    }

    // TxOut: zero value (valid — OP_RETURN outputs often carry 0 satoshis).
    #[test]
    fn txout_zero_value() {
        let txout = TxOut { value: 0, script_pubkey: vec![0x6a] }; // OP_RETURN
        let bytes = encode(&txout);
        assert_eq!(decode::<TxOut>(&bytes), txout);
    }

    // TxOut: value = u64::MAX round-trips (obviously invalid economically, but
    // the codec must not panic or corrupt the field).
    #[test]
    fn txout_max_value() {
        let txout = TxOut { value: u64::MAX, script_pubkey: vec![] };
        let bytes = encode(&txout);
        assert_eq!(decode::<TxOut>(&bytes), txout);
    }

    // TxOut: truncated script_pubkey.
    #[test]
    fn txout_truncated_script_pubkey() {
        let txout = TxOut { value: 1000, script_pubkey: vec![0x76, 0xa9, 0x14] };
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
                bits: 0,
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
                bits: 0,
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
                bits: 0,
                nonce: 0,
            },
            transactions: vec![dummy_tx; 253],
        };
        let bytes = encode(&block);
        // tx_count byte at offset 80 must be 0xfd (3-byte VarInt marker).
        assert_eq!(bytes[BlockHeader::ENCODED_LEN], 0xfd);
        assert_eq!(decode::<Block>(&bytes), block);
    }
}
