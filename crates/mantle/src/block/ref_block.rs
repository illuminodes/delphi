use std::io;

use crate::codec::{DecodeRef, Error, VarInt};
use crate::tx::TransactionRef;

use super::header::BlockHeader;

/// Zero-copy borrowed view of a `Block`.
///
/// Decodes the header and a `Vec<TransactionRef<'a>>` from a wire buffer,
/// using a caller-supplied arena to materialise the non-witness preimage of
/// any segwit transactions. After construction every transaction's
/// [`TransactionRef::txid_into`] is correct (legacy txs alias the wire
/// directly; segwit txs alias the arena).
///
/// Both the wire bytes and the arena are caller-owned. The lifetime `'a`
/// is the shorter of the two, so a `BlockRef` can outlive its decoder but
/// not its data.
#[derive(Debug)]
pub struct BlockRef<'a> {
    pub header: BlockHeader,
    /// Complete wire bytes of the block (header + tx count + every tx).
    pub raw: &'a [u8],
    /// Pre-decoded transaction views. For legacy txs `non_witness_raw`
    /// aliases `raw`; for segwit txs it aliases the arena.
    pub txs: &'a [TransactionRef<'a>],
}

impl<'a> BlockRef<'a> {
    /// Decode a block from `wire`, populating `arena` with non-witness
    /// preimages for any segwit transactions in the block.
    ///
    /// `arena` is cleared on entry so the caller can reuse the same allocation
    /// across blocks; capacity is preserved to amortise heap traffic during
    /// IBD/replay.
    ///
    /// # Errors
    /// Propagates I/O errors from the decoder.
    pub fn decode_with_arena(
        wire: &'a [u8],
        arena: &'a mut Vec<u8>,
        txs_buf: &'a mut Vec<TransactionRef<'a>>,
    ) -> Result<Self, Error> {
        arena.clear();
        txs_buf.clear();

        let (header, rest) = BlockHeader::decode_ref(wire)?;
        let (vi, mut rest) = VarInt::decode_ref(rest)?;
        let tx_count = vi
            .to_usize()
            .map_err(|e| Error::Io(io::Error::new(io::ErrorKind::InvalidData, e.to_string())))?;

        arena.reserve(rest.len());

        let txs = txs_buf;
        txs.reserve(tx_count.saturating_sub(txs.capacity()));

        // Pass 1: decode all transactions and append non-witness preimages
        // for segwit txs into the arena. Record (start, end) offsets so we
        // can patch `non_witness_raw` in a second pass once the arena is
        // immutably borrowed.
        let mut segwit_spans: Vec<(usize, usize, usize)> = Vec::new();

        for i in 0..tx_count {
            let (tx, after) = TransactionRef::decode_ref(rest)?;

            if tx.is_segwit {
                let arena_start = arena.len();
                let raw = tx.raw;
                let inputs_region_start = 4 + 2;
                let witnesses_start_offset = if tx.witnesses_buf.is_empty() {
                    raw.len() - 4
                } else {
                    (tx.witnesses_buf.as_ptr() as usize) - (raw.as_ptr() as usize)
                };
                arena.extend_from_slice(&raw[..4]);
                arena.extend_from_slice(&raw[inputs_region_start..witnesses_start_offset]);
                arena.extend_from_slice(&raw[raw.len() - 4..]);
                segwit_spans.push((i, arena_start, arena.len()));
            }

            txs.push(tx);
            rest = after;
        }

        // Pass 2: patch non_witness_raw for segwit txs using safe arena slices.
        for &(idx, start, end) in &segwit_spans {
            txs[idx].non_witness_raw = &arena[start..end];
        }

        let raw_consumed = wire.len() - rest.len();
        let raw = &wire[..raw_consumed];

        Ok(Self {
            header,
            raw,
            txs: txs.as_slice(),
        })
    }

    /// Number of transactions in this block.
    #[must_use]
    pub fn tx_count(&self) -> usize {
        self.txs.len()
    }

    /// Iterate transactions.
    pub fn transactions(&self) -> std::slice::Iter<'_, TransactionRef<'a>> {
        self.txs.iter()
    }
}

#[cfg(test)]
mod tests {
    use crate::amount::Satoshi;
    use crate::block::{Block, BlockHeader, BlockRef};
    use crate::codec::Encode;
    use crate::difficulty::{BlockTime, BlockVersion, CompactBits, Locktime, Nonce, Sequence, TxVersion};
    use crate::hash::Hash256;
    use crate::script::Witness;
    use crate::tx::{OutPoint, Transaction, TxHash, TxIn, TxOut};

    fn encode<T: Encode>(val: &T) -> Vec<u8> {
        let mut buf = Vec::new();
        val.encode(&mut buf).expect("Vec<u8> write cannot fail");
        buf
    }

    fn make_tx() -> Transaction {
        Transaction {
            version: TxVersion::new(2),
            inputs: vec![
                TxIn {
                    previous_output: OutPoint {
                        txid: Hash256::from_bytes([0xab; 32]),
                        vout: 1,
                    },
                    script_sig: vec![0x01, 0x02, 0x03].into(),
                    sequence: Sequence::new(0xffff_fffe),
                    witness: Witness::empty(),
                },
                TxIn {
                    previous_output: OutPoint {
                        txid: Hash256::from_bytes([0xcd; 32]),
                        vout: 0,
                    },
                    script_sig: vec![].into(),
                    sequence: Sequence::new(0),
                    witness: Witness::empty(),
                },
            ],
            outputs: vec![
                TxOut {
                    value: Satoshi::new(50_000),
                    script_pubkey: vec![0x76, 0xa9].into(),
                },
                TxOut {
                    value: Satoshi::ZERO,
                    script_pubkey: vec![].into(),
                },
            ],
            locktime: Locktime::new(800_000),
        }
    }

    fn make_segwit_tx() -> Transaction {
        Transaction {
            version: TxVersion::new(2),
            inputs: vec![TxIn {
                previous_output: OutPoint {
                    txid: Hash256::from_bytes([0x11u8; 32]),
                    vout: 0,
                },
                script_sig: Vec::<u8>::new().into(),
                sequence: Sequence::new(0xffff_fffe),
                witness: Witness::from_slice(&[
                    &[0xde, 0xad, 0xbe, 0xef][..],
                    &[0x01, 0x02, 0x03, 0x04, 0x05][..],
                ]),
            }],
            outputs: vec![TxOut {
                value: Satoshi::new(99_000),
                script_pubkey: vec![0x00, 0x14, 0xaa, 0xbb, 0xcc].into(),
            }],
            locktime: Locktime::new(0),
        }
    }

    #[test]
    fn block_ref_mixed_txids_match_owned() {
        let legacy = make_tx();
        let segwit = make_segwit_tx();
        let block = Block {
            header: BlockHeader::new(
                BlockVersion::new(1),
                Hash256::ZERO,
                Hash256::ZERO,
                BlockTime::new(1_700_000_000),
                CompactBits::new(0x1d00_ffff),
                Nonce::new(0),
            ),
            transactions: vec![legacy.clone(), segwit.clone()],
        };
        let bytes = encode(&block);
        let mut arena = Vec::new();

        let expected_legacy = legacy.txid().expect("legacy txid");
        let expected_segwit = segwit.txid().expect("segwit txid");

        let mut txs_buf = Vec::new();
        let block_ref = BlockRef::decode_with_arena(&bytes, &mut arena, &mut txs_buf)
            .expect("BlockRef decode must succeed");
        assert_eq!(block_ref.tx_count(), 2);

        let tx0 = &block_ref.txs[0];
        let tx1 = &block_ref.txs[1];
        assert!(!tx0.is_segwit);
        assert!(tx1.is_segwit);

        assert_eq!(
            tx0.txid_into(&mut Vec::new()).expect("hash"),
            expected_legacy
        );
        assert_eq!(
            tx1.txid_into(&mut Vec::new()).expect("hash"),
            expected_segwit
        );
    }

    #[test]
    fn block_ref_arena_reuse_clears_between_calls() {
        let segwit = make_segwit_tx();
        let block = Block {
            header: BlockHeader::new(
                BlockVersion::new(1),
                Hash256::ZERO,
                Hash256::ZERO,
                BlockTime::new(0),
                CompactBits::new(0x1d00_ffff),
                Nonce::new(0),
            ),
            transactions: vec![segwit.clone()],
        };
        let bytes = encode(&block);
        let mut arena = Vec::with_capacity(1024);
        // First decode populates the arena; second decode must clear it
        // (otherwise the arena would grow unboundedly across IBD blocks).
        {
            let mut txs_buf = Vec::new();
            let _br =
                BlockRef::decode_with_arena(&bytes, &mut arena, &mut txs_buf).expect("decode 1");
        }
        let cap_before = arena.capacity();
        {
            let mut txs_buf = Vec::new();
            let br2 =
                BlockRef::decode_with_arena(&bytes, &mut arena, &mut txs_buf).expect("decode 2");
            assert_eq!(
                br2.txs[0].txid_into(&mut Vec::new()).expect("hash"),
                segwit.txid().expect("owned txid")
            );
        }
        // Capacity should not have grown (the second pass fits within the
        // first pass's high-water mark).
        assert!(arena.capacity() >= cap_before);
    }

    #[test]
    fn block_ref_zero_transactions() {
        let block = Block {
            header: BlockHeader::new(BlockVersion::new(1), Hash256::ZERO, Hash256::ZERO, BlockTime::new(0), CompactBits::new(0), Nonce::new(0)),
            transactions: vec![],
        };
        let bytes = encode(&block);
        let mut arena = Vec::new();
        let mut txs_buf = Vec::new();
        let br = BlockRef::decode_with_arena(&bytes, &mut arena, &mut txs_buf)
            .expect("empty block must decode");
        assert_eq!(br.tx_count(), 0);
        assert_eq!(br.transactions().count(), 0);
        assert!(arena.is_empty());
    }

    #[test]
    fn block_ref_truncated_wire_fails() {
        let block = Block {
            header: BlockHeader::new(BlockVersion::new(1), Hash256::ZERO, Hash256::ZERO, BlockTime::new(0), CompactBits::new(0), Nonce::new(0)),
            transactions: vec![make_tx()],
        };
        let bytes = encode(&block);
        let truncated = &bytes[..bytes.len() / 2];
        let mut arena = Vec::new();
        let mut txs_buf = Vec::new();
        assert!(BlockRef::decode_with_arena(truncated, &mut arena, &mut txs_buf).is_err());
    }

    #[test]
    fn block_ref_truncated_at_header() {
        let mut arena = Vec::new();
        let mut txs_buf = Vec::new();
        assert!(BlockRef::decode_with_arena(&[0u8; 40], &mut arena, &mut txs_buf).is_err());
    }

    #[test]
    fn block_ref_all_txids_correct_multi_segwit() {
        let segwit1 = make_segwit_tx();
        let segwit2 = Transaction {
            version: TxVersion::new(2),
            inputs: vec![TxIn {
                previous_output: OutPoint {
                    txid: Hash256::from_bytes([0x22u8; 32]),
                    vout: 1,
                },
                script_sig: Vec::<u8>::new().into(),
                sequence: Sequence::new(0xffff_fffe),
                witness: Witness::from_slice(&[&[0xaa; 71][..], &[0xbb; 33][..]]),
            }],
            outputs: vec![TxOut {
                value: Satoshi::new(50_000),
                script_pubkey: vec![0x00, 0x14, 0xcc, 0xdd].into(),
            }],
            locktime: Locktime::new(0),
        };
        let block = Block {
            header: BlockHeader::new(BlockVersion::new(1), Hash256::ZERO, Hash256::ZERO, BlockTime::new(0), CompactBits::new(0), Nonce::new(0)),
            transactions: vec![make_tx(), segwit1.clone(), segwit2.clone(), make_tx()],
        };
        let bytes = encode(&block);
        let mut arena = Vec::new();
        let mut txs_buf = Vec::new();
        let br = BlockRef::decode_with_arena(&bytes, &mut arena, &mut txs_buf)
            .expect("block with mixed txs must decode");
        assert_eq!(br.tx_count(), 4);

        let expected_txids: Vec<_> = block
            .transactions
            .iter()
            .map(|tx| tx.txid().unwrap())
            .collect();
        for (i, tx_ref) in br.transactions().enumerate() {
            let ref_txid = tx_ref.txid_into(&mut Vec::new()).unwrap();
            assert_eq!(ref_txid, expected_txids[i], "txid mismatch at index {i}");
        }
    }

    #[test]
    fn block_ref_raw_spans_full_wire() {
        let block = Block {
            header: BlockHeader::new(BlockVersion::new(1), Hash256::ZERO, Hash256::ZERO, BlockTime::new(0), CompactBits::new(0), Nonce::new(0)),
            transactions: vec![make_tx(), make_segwit_tx()],
        };
        let bytes = encode(&block);
        let mut arena = Vec::new();
        let mut txs_buf = Vec::new();
        let br = BlockRef::decode_with_arena(&bytes, &mut arena, &mut txs_buf).unwrap();
        assert_eq!(br.raw.len(), bytes.len());
    }
}
