use std::io::{Read, Write};

use crate::codec::{Decode, Encode, Error, VarInt};
use crate::difficulty::BlockHeight;
use crate::hash::{ByteSliceExt, Hash256};
use crate::script::Witness;

use super::hash::TxHash;
use super::input::TxIn;
use super::outpoint::OutPoint;
use super::output::TxOut;

// ── Transaction ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transaction {
    pub version: i32,
    pub inputs: Vec<TxIn>,
    pub outputs: Vec<TxOut>,
    pub locktime: u32,
}

impl TxHash for Transaction {
    fn txid_into(&self, buf: &mut Vec<u8>) -> Result<Hash256, Error> {
        buf.clear();
        buf.reserve(self.base_encoded_len());
        self.encode_base(buf)?;
        Ok(buf.sha256d())
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
        let mut le = height.to_u32().to_le_bytes().to_vec();
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
            .map(|i| {
                OutPoint::ENCODED_LEN
                    + VarInt::from(i.script_sig.len()).encoded_len()
                    + i.script_sig.len()
                    + 4
            })
            .sum();
        let outputs_len: usize = self
            .outputs
            .iter()
            .map(|o| 8 + VarInt::from(o.script_pubkey.len()).encoded_len() + o.script_pubkey.len())
            .sum();
        4 // version
            + VarInt::from(self.inputs.len()).encoded_len()
            + inputs_len
            + VarInt::from(self.outputs.len()).encoded_len()
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
    fn encode_base(&self, w: &mut impl Write) -> Result<(), Error> {
        self.version.encode(w)?;
        VarInt::from(self.inputs.len()).encode(w)?;
        for input in &self.inputs {
            input.encode(w)?;
        }
        VarInt::from(self.outputs.len()).encode(w)?;
        for output in &self.outputs {
            output.encode(w)?;
        }
        self.locktime.encode(w)
    }
}

impl Encode for Transaction {
    /// BIP144 serialisation: non-witness form for legacy transactions; marker +
    /// flag + per-input witness stacks appended after outputs for segwit ones.
    fn encode(&self, w: &mut impl Write) -> Result<(), Error> {
        if !self.is_segwit() {
            return self.encode_base(w);
        }
        self.version.encode(w)?;
        w.write_all(&[0x00, 0x01])?; // marker, flag
        VarInt::from(self.inputs.len()).encode(w)?;
        for input in &self.inputs {
            input.encode(w)?;
        }
        VarInt::from(self.outputs.len()).encode(w)?;
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
    ) -> Result<Self, Error> {
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
                let n = VarInt::decode(r)?.to_usize()?;
                (true, n, None)
            } else {
                // Legacy 0-input tx: `first`=0x00 was the input count (0
                // inputs), `flag` is the first byte of the output-count VarInt.
                (false, 0usize, Some(flag))
            }
        } else {
            let n = VarInt::decode_from_first_byte(first, r)?.to_usize()?;
            (false, n, None)
        };

        if input_count > MAX_INPUTS {
            return Err(Error::LengthOverflow(input_count as u64));
        }
        let mut inputs = Vec::with_capacity(input_count);
        for _ in 0..input_count {
            inputs.push(TxIn::decode(r)?);
        }

        let output_count = if let Some(b) = legacy_output_count_first {
            VarInt::decode_from_first_byte(b, r)?.to_usize()?
        } else {
            VarInt::decode(r)?.to_usize()?
        };
        if output_count > MAX_OUTPUTS {
            return Err(Error::LengthOverflow(output_count as u64));
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
        Ok(Self {
            version,
            inputs,
            outputs,
            locktime,
        })
    }
}

impl Decode for Transaction {
    fn decode(r: &mut impl Read) -> Result<Self, Error> {
        Self::decode_limited::<10_000, 10_000>(r)
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use crate::amount::Satoshi;
    use crate::codec::{Decode, Encode, Error};
    use crate::hash::Hash256;
    use crate::script::Witness;
    use crate::tx::{OutPoint, Transaction, TxIn, TxOut};

    fn encode<T: Encode>(val: &T) -> Vec<u8> {
        let mut buf = Vec::new();
        val.encode(&mut buf).expect("Vec<u8> write cannot fail");
        buf
    }

    fn decode<T: Decode>(bytes: &[u8]) -> T {
        T::decode(&mut Cursor::new(bytes)).expect("test data must decode")
    }

    fn decode_err<T: Decode + std::fmt::Debug>(bytes: &[u8]) -> Error {
        T::decode(&mut std::io::Cursor::new(bytes)).unwrap_err()
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
    fn transaction_empty_inputs_outputs() {
        let tx = Transaction {
            version: 1,
            inputs: vec![],
            outputs: vec![],
            locktime: 0,
        };
        let bytes = encode(&tx);
        assert_eq!(decode::<Transaction>(&bytes), tx);
    }

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

    #[test]
    fn transaction_truncated_after_input_count() {
        // version(4) + VarInt(1) then nothing.
        let buf = [0x01, 0x00, 0x00, 0x00, 0x01];
        let e = decode_err::<Transaction>(&buf);
        assert!(matches!(e, Error::UnexpectedEof | Error::Io(_)));
    }

    #[test]
    fn transaction_input_count_exceeds_data() {
        // version(4) + VarInt(100) + no actual inputs.
        let mut buf = vec![0x01, 0x00, 0x00, 0x00]; // version = 1
        buf.push(0x64); // VarInt(100)
        let e = decode_err::<Transaction>(&buf);
        assert!(matches!(e, Error::UnexpectedEof | Error::Io(_)));
    }
}
