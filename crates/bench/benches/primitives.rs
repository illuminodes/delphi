use mantle::codec::{Decode, DecodeRef, Encode};
use mantle::primitives::{Block, BlockHeader, Hash256, OutPoint, Transaction, TxIn, TxInRef, TxOut, TxOutRef};
use std::hint::black_box;
use std::io::Cursor;

fn main() {
    divan::main();
}

fn encode_bytes<T: Encode>(val: &T) -> Vec<u8> {
    let mut buf = Vec::new();
    val.encode(&mut buf).unwrap();
    buf
}

fn make_header(i: usize) -> BlockHeader {
    BlockHeader {
        version: 1,
        prev_block: Hash256([i as u8; 32]),
        merkle_root: Hash256::ZERO,
        time: i as u32,
        bits: 0x1d00ffff,
        nonce: i as u32,
    }
}

fn make_tx(n_inputs: usize, n_outputs: usize) -> Transaction {
    Transaction {
        version: 1,
        inputs: (0..n_inputs)
            .map(|i| TxIn {
                previous_output: OutPoint { txid: Hash256([i as u8; 32]), vout: i as u32 },
                script_sig: vec![0x51; 20],
                sequence: u32::MAX,
            })
            .collect(),
        outputs: (0..n_outputs)
            .map(|i| TxOut {
                value: (i as u64 + 1) * 1000,
                script_pubkey: vec![0x76, 0xa9, 0x14],
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
    Hash256::decode(&mut Cursor::new([0xabu8; 32].as_slice())).unwrap()
}

#[divan::bench]
fn hash256_decode_ref() -> (Hash256, &'static [u8]) {
    Hash256::decode_ref(&[0xabu8; 32]).unwrap()
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
    bencher.bench(|| BlockHeader::decode(&mut Cursor::new(bytes.as_slice())).unwrap());
}

#[divan::bench]
fn block_header_decode_ref(bencher: divan::Bencher) {
    let bytes = encode_bytes(&make_header(42));
    bencher.bench(|| BlockHeader::decode_ref(bytes.as_slice()).unwrap());
}

// ── OutPoint ──────────────────────────────────────────────────────────────────

#[divan::bench]
fn outpoint_encode() -> Vec<u8> {
    encode_bytes(&OutPoint { txid: Hash256([0xabu8; 32]), vout: 0 })
}

#[divan::bench]
fn outpoint_decode(bencher: divan::Bencher) {
    let bytes = encode_bytes(&OutPoint { txid: Hash256([0xabu8; 32]), vout: 0 });
    bencher.bench(|| OutPoint::decode(&mut Cursor::new(bytes.as_slice())).unwrap());
}

#[divan::bench]
fn outpoint_decode_ref(bencher: divan::Bencher) {
    let bytes = encode_bytes(&OutPoint { txid: Hash256([0xabu8; 32]), vout: 0 });
    bencher.bench(|| OutPoint::decode_ref(bytes.as_slice()).unwrap());
}

// ── TxIn / TxInRef ────────────────────────────────────────────────────────────

#[divan::bench]
fn txin_encode() -> Vec<u8> {
    encode_bytes(&TxIn {
        previous_output: OutPoint { txid: Hash256([0xabu8; 32]), vout: 0 },
        script_sig: vec![0x76; 25],
        sequence: u32::MAX,
    })
}

#[divan::bench]
fn txin_decode(bencher: divan::Bencher) {
    let txin = TxIn {
        previous_output: OutPoint { txid: Hash256([0xabu8; 32]), vout: 0 },
        script_sig: vec![0x76; 25],
        sequence: u32::MAX,
    };
    let bytes = encode_bytes(&txin);
    bencher.bench(|| TxIn::decode(&mut Cursor::new(bytes.as_slice())).unwrap());
}

#[divan::bench]
fn txin_decode_ref(bencher: divan::Bencher) {
    let txin = TxIn {
        previous_output: OutPoint { txid: Hash256([0xabu8; 32]), vout: 0 },
        script_sig: vec![0x76; 25],
        sequence: u32::MAX,
    };
    let bytes = encode_bytes(&txin);
    bencher.bench(|| {
        let (r, _) = TxInRef::decode_ref(bytes.as_slice()).unwrap();
        black_box(r.sequence)
    });
}

// ── TxOut / TxOutRef ──────────────────────────────────────────────────────────

#[divan::bench]
fn txout_encode() -> Vec<u8> {
    encode_bytes(&TxOut { value: 5_000_000_000, script_pubkey: vec![0x76, 0xa9, 0x14] })
}

#[divan::bench]
fn txout_decode(bencher: divan::Bencher) {
    let bytes = encode_bytes(&TxOut { value: 5_000_000_000, script_pubkey: vec![0x76, 0xa9, 0x14] });
    bencher.bench(|| TxOut::decode(&mut Cursor::new(bytes.as_slice())).unwrap());
}

#[divan::bench]
fn txout_decode_ref(bencher: divan::Bencher) {
    let bytes = encode_bytes(&TxOut { value: 5_000_000_000, script_pubkey: vec![0x76, 0xa9, 0x14] });
    bencher.bench(|| {
        let (r, _) = TxOutRef::decode_ref(bytes.as_slice()).unwrap();
        black_box(r.value)
    });
}

// ── Transaction ───────────────────────────────────────────────────────────────

#[divan::bench(args = [1usize, 4, 16])]
fn transaction_encode(bencher: divan::Bencher, n: usize) {
    let tx = make_tx(n, n);
    bencher.bench(|| encode_bytes(&tx));
}

#[divan::bench(args = [1usize, 4, 16])]
fn transaction_decode(bencher: divan::Bencher, n: usize) {
    let bytes = encode_bytes(&make_tx(n, n));
    bencher.bench(|| Transaction::decode(&mut Cursor::new(bytes.as_slice())).unwrap());
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
    bencher.bench(|| Block::decode(&mut Cursor::new(bytes.as_slice())).unwrap());
}
