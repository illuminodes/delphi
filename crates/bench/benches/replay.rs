//! Replay-path benches: `validate_and_commit` end-to-end.
//!
//! These measure the full per-block cost that the replay loop pays:
//! UTXO prefetch from storage, `connect_block` (structure + consensus),
//! and the storage commit.  Script execution is skipped (`skip_script=true`)
//! so the numbers reflect validation overhead only — the script benches
//! in `script.rs` cover that cost in isolation.
//!
//! Workload tiers:
//! - `replay_coinbase_only` — 1-tx block, no UTXO lookups.
//! - `replay_n_spends`      — coinbase + N regular spends from prior storage.
//! - `replay_chain_k`       — K blocks committed in sequence; measures throughput.

use mantle::hash::sha256d;
use mantle::primitives::{
    Block, BlockHeader, BlockHeight, CompactBits, Hash256, OutPoint, Satoshi, Transaction, TxHash,
    TxIn, TxOut, Witness,
};
use mantle_consensus::merkle::compute_merkle_root;
use mantle_consensus::script::ScriptExecutionCache;
use mantle_storage::types::{Durability, ReqToken};
use mantle_storage::StorageHandle;
use mantle_validator::prefetched_view::HeaderSnapshot;
use mantle_validator::pipeline::validate_and_commit;
use std::hint::black_box;

fn main() {
    divan::main();
}

const EASY_BITS: CompactBits = CompactBits(0x207f_ffff);
const BASE_TIME: u32 = 1_231_006_505;

// ── Helpers ───────────────────────────────────────────────────────────────────

fn coinbase_tx(height: u32, value: u64) -> Transaction {
    let h = height.to_le_bytes();
    Transaction {
        version: 1,
        inputs: vec![TxIn {
            previous_output: OutPoint { txid: Hash256::ZERO, vout: u32::MAX },
            script_sig: vec![0x03, h[0], h[1], h[2]].into(),
            sequence: u32::MAX,
            witness: Witness::empty(),
        }],
        outputs: vec![TxOut {
            value: Satoshi::new(value),
            script_pubkey: vec![0x51].into(),
        }],
        locktime: 0,
    }
}

fn spend_tx(prev_txid: Hash256, vout: u32, value: u64) -> Transaction {
    Transaction {
        version: 1,
        inputs: vec![TxIn {
            previous_output: OutPoint { txid: prev_txid, vout },
            script_sig: vec![0x51].into(),
            sequence: u32::MAX,
            witness: Witness::empty(),
        }],
        outputs: vec![TxOut {
            value: Satoshi::new(value),
            script_pubkey: vec![0x51].into(),
        }],
        locktime: 0,
    }
}

fn mine(mut block: Block) -> Block {
    let target = block.header.bits.to_target().expect("EASY_BITS always valid");
    for nonce in 0..=u32::MAX {
        block.header.nonce = nonce;
        let mut hash = sha256d(&block.header.to_bytes());
        hash.reverse();
        if hash <= target {
            return block;
        }
    }
    panic!("no valid nonce");
}

fn make_block(prev_hash: Hash256, height: u32, txs: Vec<Transaction>) -> Block {
    let txids: Vec<Hash256> = txs.iter().map(|tx| tx.txid().expect("txid")).collect();
    let merkle_root = compute_merkle_root(&txids).expect("non-empty list has merkle root");
    mine(Block {
        header: BlockHeader {
            version: 4,
            prev_block: prev_hash,
            merkle_root,
            time: BASE_TIME + height * 600,
            bits: EASY_BITS,
            nonce: 0,
        },
        transactions: txs,
    })
}

/// Dummy header stored at height 0 under key `Hash256::ZERO`.
///
/// `median_time_past` and parent lookups for block 1 resolve `prev_block =
/// Hash256::ZERO` into this entry.  Height 0 is the "pre-genesis" slot; real
/// blocks start at height 1.
fn pre_genesis_header() -> BlockHeader {
    BlockHeader {
        version: 1,
        prev_block: Hash256::ZERO,
        merkle_root: Hash256::ZERO,
        time: BASE_TIME,
        bits: EASY_BITS,
        nonce: 0,
    }
}

/// Build a `HeaderSnapshot` that starts with the synthetic pre-genesis entry
/// at height 0 (keyed by `Hash256::ZERO`) followed by the supplied chain.
fn make_snapshot(chain: &[(Hash256, BlockHeader)]) -> HeaderSnapshot {
    let mut entries = Vec::with_capacity(chain.len() + 1);
    entries.push((Hash256::ZERO, pre_genesis_header()));
    entries.extend_from_slice(chain);
    HeaderSnapshot::new(BlockHeight(0), entries).expect("snapshot fits u32")
}

struct TmpDir(std::path::PathBuf);
impl Drop for TmpDir {
    fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); }
}

fn tmp_storage() -> (StorageHandle, TmpDir) {
    let ns = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .subsec_nanos();
    let path = std::env::temp_dir()
        .join(format!("mantle-replay-bench-{}-{}", std::process::id(), ns));
    std::fs::create_dir_all(&path).expect("create tmp dir");
    let handle = StorageHandle::open(&path).expect("StorageHandle::open");
    (handle, TmpDir(path))
}

fn commit(
    storage: &mut StorageHandle,
    block: &Block,
    height: BlockHeight,
    snap: &HeaderSnapshot,
    cache: &mut ScriptExecutionCache,
    token: u64,
) {
    validate_and_commit(block, height, storage, snap, true, Some(cache), Durability::Memory,
        ReqToken(token))
        .unwrap_or_else(|e| panic!("commit at height {}: {e}", height.0));
}

// ── Bench: single coinbase-only block ─────────────────────────────────────────

#[divan::bench]
fn replay_coinbase_only(bencher: divan::Bencher) {
    // Block 1: coinbase only, prev = Hash256::ZERO (pre-genesis).
    let block = make_block(Hash256::ZERO, 1, vec![coinbase_tx(1, 50_0000_0000)]);
    let hash = block.header.hash();
    let snap = make_snapshot(&[(hash, block.header)]);

    // Fresh storage each iteration: BIP30 would fire if we re-commit the same
    // coinbase txid into a storage that already holds it as unspent.
    bencher.bench_local(|| {
        let (mut storage, _dir) = tmp_storage();
        let mut cache = ScriptExecutionCache::new(1024);
        commit(&mut storage, black_box(&block), BlockHeight(1), &snap, &mut cache, 1);
        storage.shutdown().expect("shutdown");
    });
}

// ── Bench: block with N spends from prior UTXOs ───────────────────────────────
//
// Setup (outside timed region):
//   height   1 — fund block: coinbase with N+1 outputs (vout 0..=N)
//   heights 2-100 — empty coinbase-only blocks (satisfy coinbase maturity)
// Timed region (fresh storage each iter):
//   re-commit the entire 100-block setup chain, then bench height 101 which
//   spends the N outputs from the height-1 coinbase.

#[divan::bench(args = [1usize, 10, 50, 100])]
fn replay_n_spends(bencher: divan::Bencher, n: usize) {
    const MATURITY: u32 = 100;
    const FUND_HEIGHT: u32 = 1;
    const SPEND_HEIGHT: u32 = FUND_HEIGHT + MATURITY + 1; // 102

    // Fund block at height 1: coinbase with N outputs (one per UTXO to spend).
    let fund_cb = Transaction {
        version: 1,
        inputs: vec![TxIn {
            previous_output: OutPoint { txid: Hash256::ZERO, vout: u32::MAX },
            script_sig: vec![0x03, 0x01, 0x00, 0x00].into(),
            sequence: u32::MAX,
            witness: Witness::empty(),
        }],
        outputs: (0..n).map(|_| TxOut {
            value: Satoshi::new(1000),
            script_pubkey: vec![0x51].into(),
        }).collect(),
        locktime: 0,
    };
    let fund_block = make_block(Hash256::ZERO, FUND_HEIGHT, vec![fund_cb]);
    let fund_txid = fund_block.transactions[0].txid().expect("txid");
    let fund_hash = fund_block.header.hash();

    // Filler blocks heights 2..=SPEND_HEIGHT-1 (coinbase-only, advance the chain).
    let mut prev_hash = fund_hash;
    let mut filler: Vec<Block> = Vec::with_capacity((SPEND_HEIGHT - 2) as usize);
    for h in 2..SPEND_HEIGHT {
        let b = make_block(prev_hash, h, vec![coinbase_tx(h, 50_0000_0000)]);
        prev_hash = b.header.hash();
        filler.push(b);
    }

    // Spend block at SPEND_HEIGHT: spends the N outputs from the fund coinbase.
    let spend_txs: Vec<Transaction> = std::iter::once(coinbase_tx(SPEND_HEIGHT, 50_0000_0000))
        .chain((0..n).map(|i| spend_tx(fund_txid, u32::try_from(i).unwrap(), 999)))
        .collect();
    let spend_block = make_block(prev_hash, SPEND_HEIGHT, spend_txs);
    let spend_hash = spend_block.header.hash();

    // Build full snapshot: pre-genesis + fund + fillers + spend.
    let mut chain = vec![(fund_hash, fund_block.header)];
    for b in &filler { chain.push((b.header.hash(), b.header)); }
    chain.push((spend_hash, spend_block.header));
    let snap = make_snapshot(&chain);

    // Re-commit the entire setup chain each iteration (outside what we're
    // measuring), then bench the spend block.  Fresh storage avoids BIP30.
    bencher.bench_local(|| {
        let (mut storage, _dir) = tmp_storage();
        let mut cache = ScriptExecutionCache::new(1024);
        commit(&mut storage, &fund_block, BlockHeight(FUND_HEIGHT), &snap, &mut cache, 1);
        for (i, b) in filler.iter().enumerate() {
            commit(&mut storage, b, BlockHeight(2 + u32::try_from(i).unwrap()),
                &snap, &mut cache, 2 + i as u64);
        }
        commit(&mut storage, black_box(&spend_block), BlockHeight(SPEND_HEIGHT),
            &snap, &mut cache, u64::from(SPEND_HEIGHT));
        storage.shutdown().expect("shutdown");
    });
}

// ── Bench: K-block sequential chain ───────────────────────────────────────────

#[divan::bench(args = [10usize, 50, 100])]
fn replay_chain_k(bencher: divan::Bencher, k: usize) {
    // Pre-build K blocks starting at height 1 (prev = Hash256::ZERO = pre-genesis).
    let mut prev_hash = Hash256::ZERO;
    let mut chain: Vec<(Hash256, BlockHeader)> = Vec::with_capacity(k);
    let mut blocks: Vec<Block> = Vec::with_capacity(k);

    for i in 0..k {
        let height = u32::try_from(i + 1).expect("k fits u32");
        let block = make_block(prev_hash, height, vec![coinbase_tx(height, 50_0000_0000)]);
        let hash = block.header.hash();
        chain.push((hash, block.header));
        prev_hash = hash;
        blocks.push(block);
    }

    let snap = make_snapshot(&chain);

    bencher.bench_local(|| {
        let (mut storage, _dir) = tmp_storage();
        let mut cache = ScriptExecutionCache::new(1024);

        for (i, block) in blocks.iter().enumerate() {
            let height = BlockHeight(u32::try_from(i + 1).expect("fits"));
            commit(&mut storage, black_box(block), height, &snap, &mut cache, i as u64);
        }

        storage.shutdown().expect("shutdown");
    });
}
