use mantle::codec::{Decode, DecodeRef, Encode};
use mantle::primitives::{
    Block, BlockHeader, Hash256, OutPoint, Satoshi, Transaction, TransactionRef, TxHash, TxIn,
    TxInRef, TxOut, TxOutRef, Witness,
};
use std::hint::black_box;
use std::io::Cursor;

fn main() {
    divan::main();
}

fn encode_bytes<T: Encode>(val: &T) -> Vec<u8> {
    let mut buf = Vec::new();
    val.encode(&mut buf).expect("Vec<u8> write cannot fail");
    buf
}

fn make_header(i: usize) -> BlockHeader {
    BlockHeader {
        version: 1,
        prev_block: Hash256([u8::try_from(i).expect("bench index fits u8"); 32]),
        merkle_root: Hash256::ZERO,
        time: u32::try_from(i).expect("bench index fits u32"),
        bits: mantle::primitives::CompactBits(0x1d00_ffff),
        nonce: u32::try_from(i).expect("bench index fits u32"),
    }
}

fn make_tx(n_inputs: usize, n_outputs: usize) -> Transaction {
    Transaction {
        version: 1,
        inputs: (0..n_inputs)
            .map(|i| TxIn {
                previous_output: OutPoint {
                    txid: Hash256([u8::try_from(i).expect("bench index fits u8"); 32]),
                    vout: u32::try_from(i).expect("bench index fits u32"),
                },
                script_sig: vec![0x51; 20].into(),
                sequence: u32::MAX,
                witness: Witness::empty(),
            })
            .collect(),
        outputs: (0..n_outputs)
            .map(|i| TxOut {
                value: Satoshi::new((i as u64 + 1) * 1000),
                script_pubkey: vec![0x76, 0xa9, 0x14].into(),
            })
            .collect(),
        locktime: 0,
    }
}

// ── Hash256 ───────────────────────────────────────────────────────────────────

#[divan::bench]
fn hash256_encode() -> Vec<u8> {
    encode_bytes(&Hash256([0xabu8; 32]))
}

#[divan::bench]
fn hash256_decode() -> Hash256 {
    Hash256::decode(&mut Cursor::new([0xabu8; 32].as_slice()))
        .expect("Hash256 decode cannot fail on 32 bytes")
}

#[divan::bench]
fn hash256_decode_ref() -> (Hash256, &'static [u8]) {
    Hash256::decode_ref(&[0xabu8; 32]).expect("Hash256 decode_ref cannot fail on 32 bytes")
}

// ── BlockHeader ───────────────────────────────────────────────────────────────

#[divan::bench]
fn block_header_to_bytes() -> [u8; 80] {
    make_header(42).to_bytes()
}

#[divan::bench]
fn block_header_from_bytes(bencher: divan::Bencher) {
    let bytes = make_header(42).to_bytes();
    bencher.bench(|| BlockHeader::from_bytes(&bytes));
}

#[divan::bench]
fn block_header_encode() -> Vec<u8> {
    encode_bytes(&make_header(42))
}

#[divan::bench]
fn block_header_decode(bencher: divan::Bencher) {
    let bytes = encode_bytes(&make_header(42));
    bencher.bench(|| {
        BlockHeader::decode(&mut Cursor::new(bytes.as_slice()))
            .expect("BlockHeader decode cannot fail on valid bytes")
    });
}

#[divan::bench]
fn block_header_decode_ref(bencher: divan::Bencher) {
    let bytes = encode_bytes(&make_header(42));
    bencher.bench(|| {
        BlockHeader::decode_ref(bytes.as_slice())
            .expect("BlockHeader decode_ref cannot fail on valid bytes")
    });
}

// ── OutPoint ──────────────────────────────────────────────────────────────────

#[divan::bench]
fn outpoint_encode() -> Vec<u8> {
    encode_bytes(&OutPoint {
        txid: Hash256([0xabu8; 32]),
        vout: 0,
    })
}

#[divan::bench]
fn outpoint_decode(bencher: divan::Bencher) {
    let bytes = encode_bytes(&OutPoint {
        txid: Hash256([0xabu8; 32]),
        vout: 0,
    });
    bencher.bench(|| {
        OutPoint::decode(&mut Cursor::new(bytes.as_slice()))
            .expect("OutPoint decode cannot fail on valid bytes")
    });
}

#[divan::bench]
fn outpoint_decode_ref(bencher: divan::Bencher) {
    let bytes = encode_bytes(&OutPoint {
        txid: Hash256([0xabu8; 32]),
        vout: 0,
    });
    bencher.bench(|| {
        OutPoint::decode_ref(bytes.as_slice())
            .expect("OutPoint decode_ref cannot fail on valid bytes")
    });
}

// ── TxIn / TxInRef ────────────────────────────────────────────────────────────

#[divan::bench]
fn txin_encode() -> Vec<u8> {
    encode_bytes(&TxIn {
        previous_output: OutPoint {
            txid: Hash256([0xabu8; 32]),
            vout: 0,
        },
        script_sig: vec![0x76; 25].into(),
        sequence: u32::MAX,
        witness: Witness::empty(),
    })
}

#[divan::bench]
fn txin_decode(bencher: divan::Bencher) {
    let txin = TxIn {
        previous_output: OutPoint {
            txid: Hash256([0xabu8; 32]),
            vout: 0,
        },
        script_sig: vec![0x76; 25].into(),
        sequence: u32::MAX,
        witness: Witness::empty(),
    };
    let bytes = encode_bytes(&txin);
    bencher.bench(|| {
        TxIn::decode(&mut Cursor::new(bytes.as_slice()))
            .expect("TxIn decode cannot fail on valid bytes")
    });
}

#[divan::bench]
fn txin_decode_ref(bencher: divan::Bencher) {
    let txin = TxIn {
        previous_output: OutPoint {
            txid: Hash256([0xabu8; 32]),
            vout: 0,
        },
        script_sig: vec![0x76; 25].into(),
        sequence: u32::MAX,
        witness: Witness::empty(),
    };
    let bytes = encode_bytes(&txin);
    bencher.bench(|| {
        let (r, _) = TxInRef::decode_ref(bytes.as_slice())
            .expect("TxInRef decode_ref cannot fail on valid bytes");
        black_box(r.sequence)
    });
}

// ── TxOut / TxOutRef ──────────────────────────────────────────────────────────

#[divan::bench]
fn txout_encode() -> Vec<u8> {
    encode_bytes(&TxOut {
        value: Satoshi::new(5_000_000_000),
        script_pubkey: vec![0x76, 0xa9, 0x14].into(),
    })
}

#[divan::bench]
fn txout_decode(bencher: divan::Bencher) {
    let bytes = encode_bytes(&TxOut {
        value: Satoshi::new(5_000_000_000),
        script_pubkey: vec![0x76, 0xa9, 0x14].into(),
    });
    bencher.bench(|| {
        TxOut::decode(&mut Cursor::new(bytes.as_slice()))
            .expect("TxOut decode cannot fail on valid bytes")
    });
}

#[divan::bench]
fn txout_decode_ref(bencher: divan::Bencher) {
    let bytes = encode_bytes(&TxOut {
        value: Satoshi::new(5_000_000_000),
        script_pubkey: vec![0x76, 0xa9, 0x14].into(),
    });
    bencher.bench(|| {
        let (r, _) = TxOutRef::decode_ref(bytes.as_slice())
            .expect("TxOutRef decode_ref cannot fail on valid bytes");
        black_box(r.value)
    });
}

// ── Transaction / TransactionRef ─────────────────────────────────────────────

#[divan::bench(args = [1usize, 4, 16])]
fn transaction_encode(bencher: divan::Bencher, n: usize) {
    let tx = make_tx(n, n);
    bencher.bench(|| encode_bytes(&tx));
}

#[divan::bench(args = [1usize, 4, 16])]
fn transaction_decode(bencher: divan::Bencher, n: usize) {
    let bytes = encode_bytes(&make_tx(n, n));
    bencher.bench(|| {
        Transaction::decode(&mut Cursor::new(bytes.as_slice()))
            .expect("Transaction decode cannot fail on valid bytes")
    });
}

/// Zero-copy parse: `DecodeRef` only scans the buffer to find boundaries —
/// no Vec allocations.
#[divan::bench(args = [1usize, 4, 16])]
fn transaction_decode_ref(bencher: divan::Bencher, n: usize) {
    let bytes = encode_bytes(&make_tx(n, n));
    bencher.bench(|| {
        let (tx, _) = TransactionRef::decode_ref(bytes.as_slice())
            .expect("TransactionRef decode_ref cannot fail on valid bytes");
        black_box(tx.version)
    });
}

/// Iterate all inputs and outputs from a `TransactionRef` without allocating.
#[divan::bench(args = [1usize, 4, 16])]
fn transaction_ref_iter(bencher: divan::Bencher, n: usize) {
    let bytes = encode_bytes(&make_tx(n, n));
    let (tx_ref, _) = TransactionRef::decode_ref(bytes.as_slice())
        .expect("TransactionRef decode_ref cannot fail on valid bytes");
    bencher.bench(|| {
        let mut sum = 0u64;
        for r in tx_ref.inputs() {
            sum += u64::from(r.expect("input decode cannot fail on valid bytes").sequence);
        }
        for r in tx_ref.outputs() {
            sum += u64::from(r.expect("output decode cannot fail on valid bytes").value);
        }
        black_box(sum)
    });
}

/// Materialise a `TransactionRef` into an owned Transaction.
#[divan::bench(args = [1usize, 4, 16])]
fn transaction_ref_to_owned(bencher: divan::Bencher, n: usize) {
    let bytes = encode_bytes(&make_tx(n, n));
    let (tx_ref, _) = TransactionRef::decode_ref(bytes.as_slice())
        .expect("TransactionRef decode_ref cannot fail on valid bytes");
    bencher.bench(|| {
        tx_ref
            .to_owned()
            .expect("to_owned cannot fail on valid bytes")
    });
}

/// Re-encode a `TransactionRef` back to bytes — zero-copy read, sequential write.
#[divan::bench(args = [1usize, 4, 16])]
fn transaction_ref_encode(bencher: divan::Bencher, n: usize) {
    let bytes = encode_bytes(&make_tx(n, n));
    let (tx_ref, _) = TransactionRef::decode_ref(bytes.as_slice())
        .expect("TransactionRef decode_ref cannot fail on valid bytes");
    bencher.bench(|| encode_bytes(&tx_ref));
}

/// Compute txid via `TxHash` on a `TransactionRef` (hashes raw wire bytes, no re-encode).
#[divan::bench(args = [1usize, 4, 16])]
fn transaction_ref_txid(bencher: divan::Bencher, n: usize) {
    let bytes = encode_bytes(&make_tx(n, n));
    let (tx_ref, _) = TransactionRef::decode_ref(bytes.as_slice())
        .expect("TransactionRef decode_ref cannot fail on valid bytes");
    bencher.bench(|| tx_ref.txid().expect("txid encoding cannot fail"));
}

/// Compare: txid on owned `Transaction` vs `TransactionRef` at same sizes.
#[divan::bench(args = [1usize, 4, 16])]
fn transaction_txid(bencher: divan::Bencher, n: usize) {
    let tx = make_tx(n, n);
    bencher.bench(|| tx.txid().expect("txid encoding cannot fail"));
}

// ── Block ─────────────────────────────────────────────────────────────────────

#[divan::bench(args = [1usize, 10, 100])]
fn block_encode(bencher: divan::Bencher, n: usize) {
    let block = Block {
        header: make_header(0),
        transactions: (0..n).map(|_| make_tx(2, 2)).collect(),
    };
    bencher.bench(|| encode_bytes(&block));
}

#[divan::bench(args = [1usize, 10, 100])]
fn block_decode(bencher: divan::Bencher, n: usize) {
    let block = Block {
        header: make_header(0),
        transactions: (0..n).map(|_| make_tx(2, 2)).collect(),
    };
    let bytes = encode_bytes(&block);
    bencher.bench(|| {
        Block::decode(&mut Cursor::new(bytes.as_slice()))
            .expect("Block decode cannot fail on valid bytes")
    });
}
