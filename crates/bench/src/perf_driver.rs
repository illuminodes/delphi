/// Tight-loop perf driver — no allocator instrumentation, runs for ~5s
/// so `perf record` accumulates a statistically meaningful sample set.
use mantle::codec::Encode;
use mantle::hash::sha256d;
use mantle::primitives::{Block, BlockHeader, Hash256, OutPoint, Transaction, TxIn, TxOut};
use mantle_consensus::merkle::compute_merkle_root;
use mantle_consensus::validation::{txid, validate_block_structure};
use std::hint::black_box;
use std::time::{Duration, Instant};

fn encode<T: Encode>(v: &T) -> Vec<u8> {
    let mut b = Vec::new();
    v.encode(&mut b).unwrap();
    b
}

fn coinbase(script: Vec<u8>, value: u64) -> Transaction {
    Transaction {
        version: 1,
        inputs: vec![TxIn {
            previous_output: OutPoint { txid: Hash256::ZERO, vout: u32::MAX },
            script_sig: script,
            sequence: u32::MAX,
        }],
        outputs: vec![TxOut { value, script_pubkey: vec![0x51] }],
        locktime: 0,
    }
}

fn regular(prev: Hash256, vout: u32, value: u64) -> Transaction {
    Transaction {
        version: 1,
        inputs: vec![TxIn {
            previous_output: OutPoint { txid: prev, vout },
            script_sig: vec![0x51; 20],
            sequence: u32::MAX,
        }],
        outputs: vec![TxOut { value, script_pubkey: vec![0x51] }],
        locktime: 0,
    }
}

fn nbits_to_target(bits: u32) -> [u8; 32] {
    let exp = (bits >> 24) as usize;
    let m   = bits & 0x007f_ffff;
    let mut t = [0u8; 32];
    if exp >= 3 {
        let s = 32 - exp;
        t[s]   = ((m >> 16) & 0xff) as u8;
        t[s+1] = ((m >>  8) & 0xff) as u8;
        t[s+2] = (m         & 0xff) as u8;
    }
    t
}

fn mine(mut b: Block) -> Block {
    let target = nbits_to_target(b.header.bits);
    for n in 0..=u32::MAX {
        b.header.nonce = n;
        let mut h = sha256d(&b.header.to_bytes());
        h.reverse();
        if h <= target { return b; }
    }
    panic!("no nonce");
}

fn valid_block(txs: Vec<Transaction>) -> Block {
    let ids: Vec<Hash256> = txs.iter().map(txid).collect();
    let root = compute_merkle_root(&ids).unwrap();
    let b = Block {
        header: BlockHeader {
            version: 1, prev_block: Hash256::ZERO, merkle_root: root,
            time: 1_231_006_505, bits: 0x207f_ffff, nonce: 0,
        },
        transactions: txs,
    };
    mine(b)
}

fn run_for(dur: Duration, mut f: impl FnMut()) -> u64 {
    let deadline = Instant::now() + dur;
    let mut n = 0u64;
    while Instant::now() < deadline { f(); n += 1; }
    n
}

fn main() {
    let secs = Duration::from_secs(2);

    // ── workload A: validate_block_structure, 100-tx block ───────────────────
    {
        let mut txs = vec![coinbase(vec![0x03, 0x64, 0x00, 0x00], 5_000_000_000)];
        for i in 1..100u8 { txs.push(regular(Hash256([i; 32]), 0, 1000)); }
        let block = valid_block(txs);
        let n = run_for(secs, || { validate_block_structure(black_box(&block)).unwrap(); black_box(()); });
        eprintln!("validate_structure(100tx): {n} iters");
    }

    // ── workload B: txid over a 16-input transaction ─────────────────────────
    {
        let tx = Transaction {
            version: 1,
            inputs: (0..16u8).map(|i| TxIn {
                previous_output: OutPoint { txid: Hash256([i; 32]), vout: u32::from(i) },
                script_sig: vec![0x51; 20],
                sequence: u32::MAX,
            }).collect(),
            outputs: vec![TxOut { value: 1000, script_pubkey: vec![0x51] }],
            locktime: 0,
        };
        let n = run_for(secs, || { black_box(txid(black_box(&tx))); });
        eprintln!("txid(16-input tx): {n} iters");
    }

    // ── workload C: compute_merkle_root over 1000 txids ──────────────────────
    {
        let ids: Vec<Hash256> = (0..1000u32).map(|i| Hash256(sha256d(&i.to_le_bytes()))).collect();
        let n = run_for(secs, || { black_box(compute_merkle_root(black_box(&ids))); });
        eprintln!("merkle_root(1000): {n} iters");
    }

    // ── workload D: block encode + decode roundtrip, 50 txs ──────────────────
    {
        use mantle::codec::Decode;
        use std::io::Cursor;
        let mut txs = vec![coinbase(vec![0x03, 0x32, 0x00, 0x00], 5_000_000_000)];
        for i in 1..50u8 { txs.push(regular(Hash256([i; 32]), 0, 1000)); }
        let block = valid_block(txs);
        let bytes = encode(&block);
        let n = run_for(secs, || {
            black_box(Block::decode(&mut Cursor::new(black_box(bytes.as_slice()))).unwrap());
        });
        eprintln!("block_decode(50tx): {n} iters");
    }
}
