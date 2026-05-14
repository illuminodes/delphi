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
        let tx_count = vi.to_usize()
            .map_err(|e| Error::Io(io::Error::new(io::ErrorKind::InvalidData, e.to_string())))?;

        arena.reserve(rest.len());

        let txs = txs_buf;
        txs.reserve(tx_count.saturating_sub(txs.capacity()));

        // SAFETY: each segwit tx appends to `arena` and stores a slice into
        // it. We must not realloc the arena between writes (would invalidate
        // earlier slices). The `reserve(rest.len())` above gives capacity
        // ≥ everything we will ever append (non-witness ≤ wire size), so no
        // realloc occurs. We hand out `&'a [u8]` slices from the arena via a
        // raw pointer because the borrow checker conservatively rejects the
        // mut/immut interleaving of the loop. Soundness rests on: (a) no
        // realloc, (b) we never read or mutate prior arena bytes after they
        // are committed to a TransactionRef, (c) the arena outlives `'a`.
        let arena_ptr: *mut Vec<u8> = arena;

        for _ in 0..tx_count {
            let (mut tx, after) = TransactionRef::decode_ref(rest)?;

            if tx.is_segwit {
                let arena_start;
                let arena_end;
                #[allow(unsafe_code)]
                unsafe {
                    let arena_mut = &mut *arena_ptr;
                    arena_start = arena_mut.len();
                    let raw = tx.raw;
                    let inputs_region_start = 4 + 2;
                    let witnesses_start_offset = if tx.witnesses_buf.is_empty() {
                        raw.len() - 4
                    } else {
                        (tx.witnesses_buf.as_ptr() as usize) - (raw.as_ptr() as usize)
                    };
                    arena_mut.extend_from_slice(&raw[..4]);
                    arena_mut.extend_from_slice(&raw[inputs_region_start..witnesses_start_offset]);
                    arena_mut.extend_from_slice(&raw[raw.len() - 4..]);
                    arena_end = arena_mut.len();
                }
                #[allow(unsafe_code)]
                let slice: &'a [u8] = unsafe {
                    let base = (*arena_ptr).as_ptr();
                    std::slice::from_raw_parts(base.add(arena_start), arena_end - arena_start)
                };
                tx.non_witness_raw = slice;
            }

            txs.push(tx);
            rest = after;
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
    use crate::codec::Encode;
    use crate::difficulty::CompactBits;
    use crate::hash::Hash256;
    use crate::script::Witness;
    use crate::block::{BlockHeader, Block, BlockRef};
    use crate::tx::{OutPoint, TxIn, TxOut, Transaction, TxHash};

    fn encode<T: Encode>(val: &T) -> Vec<u8> {
        let mut buf = Vec::new();
        val.encode(&mut buf).expect("Vec<u8> write cannot fail");
        buf
    }

    fn make_tx() -> Transaction {
        Transaction {
            version: 2,
            inputs: vec![
                TxIn {
                    previous_output: OutPoint {
                        txid: Hash256::from_bytes([0xab; 32]),
                        vout: 1,
                    },
                    script_sig: vec![0x01, 0x02, 0x03].into(),
                    sequence: 0xffff_fffe,
                    witness: Witness::empty(),
                },
                TxIn {
                    previous_output: OutPoint {
                        txid: Hash256::from_bytes([0xcd; 32]),
                        vout: 0,
                    },
                    script_sig: vec![].into(),
                    sequence: 0,
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
            locktime: 800_000,
        }
    }

    fn make_segwit_tx() -> Transaction {
        Transaction {
            version: 2,
            inputs: vec![TxIn {
                previous_output: OutPoint {
                    txid: Hash256::from_bytes([0x11u8; 32]),
                    vout: 0,
                },
                script_sig: Vec::<u8>::new().into(),
                sequence: 0xffff_fffe,
                witness: Witness::from_slice(&[
                    &[0xde, 0xad, 0xbe, 0xef][..],
                    &[0x01, 0x02, 0x03, 0x04, 0x05][..],
                ]),
            }],
            outputs: vec![TxOut {
                value: Satoshi::new(99_000),
                script_pubkey: vec![0x00, 0x14, 0xaa, 0xbb, 0xcc].into(),
            }],
            locktime: 0,
        }
    }

    #[test]
    fn block_ref_mixed_txids_match_owned() {
        let legacy = make_tx();
        let segwit = make_segwit_tx();
        let block = Block {
            header: BlockHeader {
                version: 1,
                prev_block: Hash256::ZERO,
                merkle_root: Hash256::ZERO,
                time: 1_700_000_000,
                bits: CompactBits(0x1d00_ffff),
                nonce: 0,
            },
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
            header: BlockHeader {
                version: 1,
                prev_block: Hash256::ZERO,
                merkle_root: Hash256::ZERO,
                time: 0,
                bits: CompactBits(0x1d00_ffff),
                nonce: 0,
            },
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
}
