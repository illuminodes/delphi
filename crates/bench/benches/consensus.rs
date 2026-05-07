use mantle::hash::sha256d;
use mantle::primitives::{Block, BlockHeader, BlockHeight, CompactBits, Hash256, OutPoint, Transaction, TxHash, TxIn, TxOut};
use mantle_consensus::chain_view::{ChainView, Coin, MemoryChainView};
use mantle_consensus::merkle::compute_merkle_root;
use mantle::codec::Encode;
use mantle_consensus::validation::{FixedClock, validate_block_structure, validate_block_wire, validate_block_context_with_clock};
use std::hint::black_box;

fn main() {
    divan::main();
}

// ── Helpers ───────────────────────────────────────────────────────────────────

const EASY_BITS: CompactBits = CompactBits(0x207f_ffff);

fn coinbase_tx(value: u64) -> Transaction {
    Transaction {
        version: 1,
        inputs: vec![TxIn {
            previous_output: OutPoint { txid: Hash256::ZERO, vout: u32::MAX },
            script_sig: vec![0x03, 0x01, 0x00, 0x00], // BIP34 height=1
            sequence: u32::MAX,
        }],
        outputs: vec![TxOut { value, script_pubkey: vec![0x51] }],
        locktime: 0,
    }
}

fn regular_tx(prev: Hash256, vout: u32, value: u64) -> Transaction {
    Transaction {
        version: 1,
        inputs: vec![TxIn {
            previous_output: OutPoint { txid: prev, vout },
            script_sig: vec![0x51],
            sequence: u32::MAX,
        }],
        outputs: vec![TxOut { value, script_pubkey: vec![0x51] }],
        locktime: 0,
    }
}

fn mine_block(mut block: Block) -> Block {
    let target = block.header.bits.to_target().expect("EASY_BITS must produce a valid target");
    for nonce in 0..=u32::MAX {
        block.header.nonce = nonce;
        let raw = sha256d(&block.header.to_bytes());
        let mut be = raw;
        be.reverse();
        if be <= target {
            return block;
        }
    }
    panic!("no valid nonce found");
}

fn make_valid_block(txs: Vec<Transaction>) -> Block {
    let txids: Vec<Hash256> = txs.iter().map(|tx| tx.txid().expect("txid encoding cannot fail")).collect();
    let merkle_root = compute_merkle_root(&txids).expect("non-empty tx list must have a merkle root");
    let block = Block {
        header: BlockHeader {
            version: 1,
            prev_block: Hash256::ZERO,
            merkle_root,
            time: 1_231_006_505,
            bits: EASY_BITS,
            nonce: 0,
        },
        transactions: txs,
    };
    mine_block(block)
}

fn build_chain(len: u32) -> MemoryChainView {
    let mut view = MemoryChainView::new();
    let mut prev = Hash256::ZERO;
    for i in 0..len {
        let h = BlockHeader {
            version: 4,
            prev_block: prev,
            merkle_root: Hash256::ZERO,
            time: 1_296_688_602 + i * 600,
            bits: EASY_BITS,
            nonce: 0,
        };
        prev = Hash256(sha256d(&h.to_bytes()));
        view.push_header(h);
    }
    view
}

// ── txid ──────────────────────────────────────────────────────────────────────

#[divan::bench]
fn txid_coinbase() {
    black_box(coinbase_tx(5_000_000_000).txid().expect("txid encoding cannot fail"));
}

#[divan::bench(args = [1usize, 4, 16])]
fn txid_regular(bencher: divan::Bencher, n_inputs: usize) {
    let tx = regular_tx(Hash256([0xabu8; 32]), 0, 1000);
    // Build a tx with n_inputs inputs to vary serialisation size.
    let big_tx = Transaction {
        inputs: (0..n_inputs)
            .map(|i| TxIn {
                previous_output: OutPoint { txid: Hash256([u8::try_from(i).expect("bench index fits u8"); 32]), vout: u32::try_from(i).expect("bench index fits u32") },
                script_sig: vec![0x51; 20],
                sequence: u32::MAX,
            })
            .collect(),
        ..tx
    };
    bencher.bench(|| black_box(big_tx.txid()));
}

// ── compute_merkle_root ───────────────────────────────────────────────────────

#[divan::bench(args = [1usize, 2, 3, 7, 8, 100, 2000])]
fn merkle_root(bencher: divan::Bencher, n: usize) {
    let txids: Vec<Hash256> = (0..n).map(|i| Hash256([u8::try_from(i).expect("bench index fits u8"); 32])).collect();
    bencher.bench(|| compute_merkle_root(black_box(&txids)));
}

// ── validate_block_structure ──────────────────────────────────────────────────

#[divan::bench]
fn validate_structure_single_coinbase(bencher: divan::Bencher) {
    let block = make_valid_block(vec![coinbase_tx(5_000_000_000)]);
    bencher.bench(|| validate_block_structure(black_box(&block)).expect("valid block must pass structure validation"));
}

#[divan::bench(args = [2usize, 10, 100])]
fn validate_structure_n_txs(bencher: divan::Bencher, n: usize) {
    let mut txs = vec![coinbase_tx(5_000_000_000)];
    for i in 1..n {
        txs.push(regular_tx(Hash256([u8::try_from(i).expect("bench index fits u8"); 32]), 0, 1000));
    }
    let block = make_valid_block(txs);
    bencher.bench(|| validate_block_structure(black_box(&block)).expect("valid block must pass structure validation"));
}

// ── validate_block_wire ───────────────────────────────────────────────────────

#[divan::bench]
fn validate_wire_single_coinbase(bencher: divan::Bencher) {
    let block = make_valid_block(vec![coinbase_tx(5_000_000_000)]);
    let mut wire = Vec::new();
    block.encode(&mut wire).expect("Vec<u8> write cannot fail");
    let mut t = Vec::new();
    let mut si = std::collections::HashSet::new();
    let mut st = std::collections::HashSet::new();
    bencher.bench_local(|| {
        validate_block_wire(black_box(&wire), &mut t, &mut si, &mut st).expect("valid block wire must pass validation");
    });
}

#[divan::bench(args = [2usize, 10, 100])]
fn validate_wire_n_txs(bencher: divan::Bencher, n: usize) {
    let mut txs = vec![coinbase_tx(5_000_000_000)];
    for i in 1..n {
        txs.push(regular_tx(Hash256([u8::try_from(i).expect("bench index fits u8"); 32]), 0, 1000));
    }
    let block = make_valid_block(txs);
    let mut wire = Vec::new();
    block.encode(&mut wire).expect("Vec<u8> write cannot fail");
    let mut t = Vec::new();
    let mut si = std::collections::HashSet::new();
    let mut st = std::collections::HashSet::new();
    bencher.bench_local(|| {
        validate_block_wire(black_box(&wire), &mut t, &mut si, &mut st).expect("valid block wire must pass validation");
    });
}

// ── validate_block_context ────────────────────────────────────────────────────

#[divan::bench]
fn validate_context_chain_12(bencher: divan::Bencher) {
    let view = build_chain(12);
    let tip = view.tip();
    let parent_time = view.get_header_at(tip.height)
        .expect("storage error")
        .expect("tip header must exist")
        .time;
    let clock_now = u64::from(parent_time) + 600;

    let tx = Transaction {
        version: 4,
        inputs: vec![TxIn {
            previous_output: OutPoint { txid: Hash256::ZERO, vout: u32::MAX },
            script_sig: vec![0x03, 0x0d, 0x00, 0x00], // height=13
            sequence: u32::MAX,
        }],
        outputs: vec![TxOut { value: 5_000_000_000, script_pubkey: vec![0x51] }],
        locktime: 0,
    };
    let merkle_root = compute_merkle_root(&[tx.txid().expect("txid encoding cannot fail")]).expect("single-tx merkle root must exist");
    let mtp = view.median_time_past(&BlockHeader {
        version: 4,
        prev_block: tip.hash,
        merkle_root: Hash256::ZERO,
        time: 0,
        bits: EASY_BITS,
        nonce: 0,
    }).expect("median_time_past must succeed on valid chain");
    let block = Block {
        header: BlockHeader {
            version: 4,
            prev_block: tip.hash,
            merkle_root,
            time: mtp + 1,
            bits: EASY_BITS,
            nonce: 0,
        },
        transactions: vec![tx],
    };
    let clock = FixedClock(clock_now + 3600);
    bencher.bench(|| {
        validate_block_context_with_clock(black_box(&block), &view, &clock).expect("valid block must pass context validation");
    });
}

// ── MemoryChainView lookups ───────────────────────────────────────────────────

#[divan::bench]
fn chain_view_get_header_by_hash(bencher: divan::Bencher) {
    let view = build_chain(100);
    let tip = view.tip();
    bencher.bench(|| view.get_header(black_box(&tip.hash)).expect("storage error"));
}

#[divan::bench]
fn chain_view_get_header_at(bencher: divan::Bencher) {
    let view = build_chain(100);
    bencher.bench(|| view.get_header_at(black_box(BlockHeight(50))).expect("storage error"));
}

#[divan::bench]
fn chain_view_height_of(bencher: divan::Bencher) {
    let view = build_chain(100);
    let tip = view.tip();
    bencher.bench(|| view.height_of(black_box(&tip.hash)).expect("storage error"));
}

#[divan::bench]
fn chain_view_tip(bencher: divan::Bencher) {
    let view = build_chain(100);
    bencher.bench(|| black_box(view.tip()));
}

#[divan::bench]
fn chain_view_get_coin_hit(bencher: divan::Bencher) {
    let mut view = MemoryChainView::new();
    let op = OutPoint { txid: Hash256([1u8; 32]), vout: 0 };
    view.insert_coin(op, Coin { value: 5_000_000_000, script_pubkey: vec![0x51], height: BlockHeight(1), coinbase: false });
    bencher.bench(|| view.get_coin(black_box(&op)).expect("storage error"));
}

#[divan::bench]
fn chain_view_get_coin_miss(bencher: divan::Bencher) {
    let view = MemoryChainView::new();
    let op = OutPoint { txid: Hash256([0xffu8; 32]), vout: 0 };
    bencher.bench(|| view.get_coin(black_box(&op)).expect("storage error"));
}

#[divan::bench(args = [1usize, 10, 100])]
fn chain_view_get_coins_batch(bencher: divan::Bencher, n: usize) {
    let mut view = MemoryChainView::new();
    let ops: Vec<OutPoint> = (0..n)
        .map(|i| OutPoint { txid: Hash256([u8::try_from(i).expect("bench index fits u8"); 32]), vout: 0 })
        .collect();
    for &op in &ops {
        view.insert_coin(op, Coin { value: 1000, script_pubkey: vec![], height: BlockHeight(1), coinbase: false });
    }
    bencher.bench(|| view.get_coins(black_box(&ops)).expect("storage error"));
}

#[divan::bench]
fn chain_view_median_time_past(bencher: divan::Bencher) {
    let view = build_chain(12);
    let tip = view.tip();
    let next_header = BlockHeader {
        version: 4,
        prev_block: tip.hash,
        merkle_root: Hash256::ZERO,
        time: 0,
        bits: EASY_BITS,
        nonce: 0,
    };
    bencher.bench(|| view.median_time_past(black_box(&next_header)).expect("median_time_past must succeed on valid chain"));
}
