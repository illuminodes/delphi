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
        Ok((Self(head.try_into().map_err(|_| io::Error::from(io::ErrorKind::InvalidData))?), tail))
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

impl BlockHeader {
    /// Serialize to a fixed 80-byte stack buffer.
    #[must_use]
    #[allow(clippy::missing_panics_doc)] // writing into a fixed-size array cannot fail
    pub fn to_bytes(&self) -> [u8; Self::ENCODED_LEN] {
        let mut buf = [0u8; Self::ENCODED_LEN];
        self.encode(&mut buf.as_mut_slice()).expect("fixed-size write cannot fail");
        buf
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
        let bits = u32::decode(&mut r)?;
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

impl Transaction {
    /// Serialised byte length of this transaction, computed arithmetically
    /// without allocating. Matches what `Encode::encode` would produce.
    #[must_use]
    pub fn encoded_len(&self) -> usize {
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
        4 // version (i32)
            + VarInt(self.inputs.len() as u64).encoded_len()
            + inputs_len
            + VarInt(self.outputs.len() as u64).encoded_len()
            + outputs_len
            + 4 // locktime (u32)
    }
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
#[derive(Debug, Clone, Copy)]
pub struct TransactionRef<'a> {
    pub version: i32,
    pub locktime: u32,
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

    /// Serialised byte length, computed without allocating.
    #[must_use]
    pub fn encoded_len(&self) -> usize {
        let inputs_len: usize = self
            .inputs()
            .map(|r| r.map_or(0, |i| OutPoint::ENCODED_LEN + VarInt(i.script_sig.len() as u64).encoded_len() + i.script_sig.len() + 4))
            .sum();
        let outputs_len: usize = self
            .outputs()
            .map(|r| r.map_or(0, |o| 8 + VarInt(o.script_pubkey.len() as u64).encoded_len() + o.script_pubkey.len()))
            .sum();
        4
            + VarInt(self.input_count as u64).encoded_len()
            + inputs_len
            + VarInt(self.output_count as u64).encoded_len()
            + outputs_len
            + 4
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
                    script_sig: i.script_sig.to_vec(),
                    sequence: i.sequence,
                })
            })
            .collect::<io::Result<Vec<_>>>()?;
        let outputs = self
            .outputs()
            .map(|r| {
                r.map(|o| TxOut {
                    value: o.value,
                    script_pubkey: o.script_pubkey.to_vec(),
                })
            })
            .collect::<io::Result<Vec<_>>>()?;
        Ok(Transaction { version: self.version, inputs, outputs, locktime: self.locktime })
    }
}

impl Encode for TransactionRef<'_> {
    fn encode(&self, w: &mut impl Write) -> io::Result<()> {
        self.version.encode(w)?;
        VarInt(self.input_count as u64).encode(w)?;
        for r in self.inputs() {
            let i = r?;
            i.previous_output.encode(w)?;
            VarInt(i.script_sig.len() as u64).encode(w)?;
            w.write_all(i.script_sig)?;
            i.sequence.encode(w)?;
        }
        VarInt(self.output_count as u64).encode(w)?;
        for r in self.outputs() {
            let o = r?;
            o.value.encode(w)?;
            VarInt(o.script_pubkey.len() as u64).encode(w)?;
            w.write_all(o.script_pubkey)?;
        }
        self.locktime.encode(w)
    }
}

impl<'a> DecodeRef<'a> for TransactionRef<'a> {
    fn decode_ref(buf: &'a [u8]) -> io::Result<(Self, &'a [u8])> {
        let (version, buf) = i32::decode_ref(buf)?;

        let (VarInt(n), buf) = VarInt::decode_ref(buf)?;
        let input_count = usize::try_from(n)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        let inputs_buf = buf;
        // Scan past inputs to find the outputs boundary.
        let mut rest = buf;
        for _ in 0..input_count {
            let (_, r) = TxInRef::decode_ref(rest)?;
            rest = r;
        }

        let (VarInt(m), rest) = VarInt::decode_ref(rest)?;
        let output_count = usize::try_from(m)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        let outputs_buf = rest;
        // Scan past outputs to reach locktime.
        let mut rest = rest;
        for _ in 0..output_count {
            let (_, r) = TxOutRef::decode_ref(rest)?;
            rest = r;
        }

        let (locktime, rest) = u32::decode_ref(rest)?;
        Ok((
            Self { version, locktime, input_count, inputs_buf, output_count, outputs_buf },
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

    // ── TransactionRef ────────────────────────────────────────────────────────

    fn make_tx() -> Transaction {
        Transaction {
            version: 2,
            inputs: vec![
                TxIn {
                    previous_output: OutPoint { txid: Hash256([0xab; 32]), vout: 1 },
                    script_sig: vec![0x01, 0x02, 0x03],
                    sequence: 0xffff_fffe,
                },
                TxIn {
                    previous_output: OutPoint { txid: Hash256([0xcd; 32]), vout: 0 },
                    script_sig: vec![],
                    sequence: 0,
                },
            ],
            outputs: vec![
                TxOut { value: 50_000, script_pubkey: vec![0x76, 0xa9] },
                TxOut { value: 0, script_pubkey: vec![] },
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
