//! Tight-loop perf driver for the replay path.
//!
//! Runs for ~5 s per workload so `perf record` accumulates a statistically
//! meaningful sample set.  No allocator shim — numbers reflect real throughput.
//!
//! Usage:
//!   cargo build --release --bin replay-perf
//!   perf record -g ./target/release/replay-perf
//!   perf report
#![allow(clippy::cast_precision_loss)]

use std::hint::black_box;
use std::time::{Duration, Instant};

use mantle::primitives::{
    Block, BlockHeader, BlockHeight, CompactBits, Hash256, Locktime, OutPoint, Satoshi, Sequence,
    Transaction, TxHash, TxIn, TxOut, TxVersion, Witness,
};
use mantle_consensus::merkle::compute_merkle_root;
use mantle_consensus::script::ScriptExecutionCache;
use mantle_storage::StorageHandle;
use mantle_storage::types::{Durability, ReqToken};
use mantle_validator::{HeaderSnapshot, PipelineOpts, ReplayBufs};

const EASY_BITS: CompactBits = CompactBits::new(0x207f_ffff);
const BASE_TIME: u32 = 1_231_006_505;
const SECS: Duration = Duration::from_secs(5);

fn coinbase_tx(height: u32, value: u64) -> Transaction {
    let h = height.to_le_bytes();
    Transaction {
        version: TxVersion::new(1),
        inputs: vec![TxIn {
            previous_output: OutPoint {
                txid: Hash256::ZERO,
                vout: u32::MAX,
            },
            script_sig: vec![0x03, h[0], h[1], h[2]].into(),
            sequence: Sequence::MAX,
            witness: Witness::empty(),
        }],
        outputs: vec![TxOut {
            value: Satoshi::new(value),
            script_pubkey: vec![0x51].into(),
        }],
        locktime: Locktime::new(0),
    }
}

fn spend_tx(prev_txid: Hash256, vout: u32, value: u64) -> Transaction {
    Transaction {
        version: TxVersion::new(1),
        inputs: vec![TxIn {
            previous_output: OutPoint {
                txid: prev_txid,
                vout,
            },
            script_sig: vec![0x51].into(),
            sequence: Sequence::MAX,
            witness: Witness::empty(),
        }],
        outputs: vec![TxOut {
            value: Satoshi::new(value),
            script_pubkey: vec![0x51].into(),
        }],
        locktime: Locktime::new(0),
    }
}

fn mine(mut block: Block) -> Block {
    let target = block
        .header
        .bits()
        .to_target()
        .expect("EASY_BITS always valid");
    for nonce in 0..=u32::MAX {
        block.header.set_nonce(nonce);
        let mut hash = *Hash256::hash(&block.header.to_bytes()).as_bytes();
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
        header: BlockHeader::new(
            4,
            prev_hash,
            merkle_root,
            BASE_TIME + height * 600,
            EASY_BITS,
            0,
        ),
        transactions: txs,
    })
}

fn pre_genesis_header() -> BlockHeader {
    BlockHeader::new(1, Hash256::ZERO, Hash256::ZERO, BASE_TIME, EASY_BITS, 0)
}

fn make_snapshot(chain: &[(Hash256, BlockHeader)]) -> HeaderSnapshot {
    let mut entries = Vec::with_capacity(chain.len() + 1);
    entries.push((Hash256::ZERO, pre_genesis_header()));
    entries.extend_from_slice(chain);
    HeaderSnapshot::new(BlockHeight::new(0), entries).expect("snapshot fits u32")
}

struct TmpDir(std::path::PathBuf);
impl Drop for TmpDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn tmp_storage() -> (StorageHandle<'static>, TmpDir) {
    let ns = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .subsec_nanos();
    let path =
        std::env::temp_dir().join(format!("mantle-replay-perf-{}-{}", std::process::id(), ns));
    std::fs::create_dir_all(&path).expect("create tmp dir");
    let handle = StorageHandle::open(&path).expect("StorageHandle::open");
    (handle, TmpDir(path))
}

fn commit(
    bufs: &mut ReplayBufs,
    storage: &mut StorageHandle<'static>,
    block: &Block,
    height: BlockHeight,
    snap: &HeaderSnapshot,
    cache: &mut ScriptExecutionCache,
    token: u64,
) {
    bufs.validate_and_commit(
        block,
        height,
        storage,
        snap,
        PipelineOpts {
            skip_script: true,
            exec_cache: Some(cache),
            durability: Durability::Memory,
            token: ReqToken(token),
        },
    )
    .unwrap_or_else(|e| panic!("commit at height {}: {e}", height.to_u32()));
}

fn run_for(dur: Duration, mut f: impl FnMut()) -> u64 {
    let deadline = Instant::now() + dur;
    let mut n = 0u64;
    while Instant::now() < deadline {
        f();
        n += 1;
    }
    n
}

#[allow(clippy::too_many_lines)]
fn main() {
    // ── workload A: coinbase-only block (no UTXO lookups) ────────────────────
    // Fresh storage each iter to avoid BIP30 on repeated coinbase txid.
    {
        let block = make_block(Hash256::ZERO, 1, vec![coinbase_tx(1, 50_0000_0000)]);
        let hash = block.header.hash();
        let snap = make_snapshot(&[(hash, block.header)]);

        let n = run_for(SECS, || {
            let (mut storage, _dir) = tmp_storage();
            let mut cache = ScriptExecutionCache::new(1024);
            let mut bufs = ReplayBufs::new();
            commit(
                &mut bufs,
                &mut storage,
                black_box(&block),
                BlockHeight::new(1),
                &snap,
                &mut cache,
                1,
            );
            storage.shutdown().expect("shutdown");
        });
        eprintln!(
            "replay_coinbase_only: {n} iters  ({:.0} blocks/s)",
            n as f64 / SECS.as_secs_f64()
        );
    }

    // ── workload B: block with 50 UTXO spends ───────────────────────────────
    // Fund coinbase at height 1 with 50 outputs; 100 filler blocks for maturity;
    // spend block at height 102.  Setup chain re-committed each iter.
    {
        const N_SPENDS: usize = 50;
        const MATURITY: u32 = 100;
        const FUND_H: u32 = 1;
        const SPEND_H: u32 = FUND_H + MATURITY + 1;

        let fund_cb = Transaction {
            version: TxVersion::new(1),
            inputs: vec![TxIn {
                previous_output: OutPoint {
                    txid: Hash256::ZERO,
                    vout: u32::MAX,
                },
                script_sig: vec![0x03, 0x01, 0x00, 0x00].into(),
                sequence: Sequence::MAX,
                witness: Witness::empty(),
            }],
            outputs: (0..N_SPENDS)
                .map(|_| TxOut {
                    value: Satoshi::new(1000),
                    script_pubkey: vec![0x51].into(),
                })
                .collect(),
            locktime: Locktime::new(0),
        };
        let fund_block = make_block(Hash256::ZERO, FUND_H, vec![fund_cb]);
        let fund_txid = fund_block.transactions[0].txid().expect("txid");
        let fund_hash = fund_block.header.hash();

        let mut prev_hash = fund_hash;
        let mut filler: Vec<Block> = Vec::new();
        for h in 2..SPEND_H {
            let b = make_block(prev_hash, h, vec![coinbase_tx(h, 50_0000_0000)]);
            prev_hash = b.header.hash();
            filler.push(b);
        }

        let spend_txs: Vec<Transaction> = std::iter::once(coinbase_tx(SPEND_H, 50_0000_0000))
            .chain((0..N_SPENDS).map(|i| spend_tx(fund_txid, u32::try_from(i).expect("fits"), 999)))
            .collect();
        let spend_block = make_block(prev_hash, SPEND_H, spend_txs);
        let spend_hash = spend_block.header.hash();

        let mut chain = vec![(fund_hash, fund_block.header)];
        for b in &filler {
            chain.push((b.header.hash(), b.header));
        }
        chain.push((spend_hash, spend_block.header));
        let snap = make_snapshot(&chain);

        let mut iter_idx = 0u32;
        let workload_start = Instant::now();
        eprintln!("[B] workload begin t=0");
        let n = run_for(SECS, || {
            let it_start = Instant::now();
            eprint!("[B] {iter_idx} ");
            let (mut storage, _dir) = tmp_storage();
            let mut cache = ScriptExecutionCache::new(1024);
            let mut bufs = ReplayBufs::new();
            eprint!("F ");
            commit(
                &mut bufs,
                &mut storage,
                &fund_block,
                BlockHeight::new(FUND_H),
                &snap,
                &mut cache,
                1,
            );
            for (i, b) in filler.iter().enumerate() {
                eprint!("f{i} ");
                commit(
                    &mut bufs,
                    &mut storage,
                    b,
                    BlockHeight::new(2 + u32::try_from(i).expect("fits")),
                    &snap,
                    &mut cache,
                    2 + i as u64,
                );
            }
            eprint!("S ");
            commit(
                &mut bufs,
                &mut storage,
                black_box(&spend_block),
                BlockHeight::new(SPEND_H),
                &snap,
                &mut cache,
                u64::from(SPEND_H),
            );
            eprint!("X ");
            storage.shutdown().expect("shutdown");
            let dt = it_start.elapsed();
            eprintln!(
                "dt={:.1} wall={:.2}",
                dt.as_secs_f64() * 1000.0,
                workload_start.elapsed().as_secs_f64()
            );
            iter_idx += 1;
        });
        eprintln!(
            "[B] workload end wall={:.2}s n={n}",
            workload_start.elapsed().as_secs_f64()
        );
        eprintln!(
            "replay_50_spends:     {n} iters  ({:.0} blocks/s)",
            n as f64 / SECS.as_secs_f64()
        );
    }

    // ── workload C: 100-block chain, sequential ──────────────────────────────
    {
        const CHAIN_LEN: u32 = 100;

        let mut prev_hash = Hash256::ZERO;
        let mut chain: Vec<(Hash256, BlockHeader)> = Vec::with_capacity(CHAIN_LEN as usize);
        let mut blocks: Vec<Block> = Vec::with_capacity(CHAIN_LEN as usize);

        for i in 1..=CHAIN_LEN {
            let block = make_block(prev_hash, i, vec![coinbase_tx(i, 50_0000_0000)]);
            let hash = block.header.hash();
            chain.push((hash, block.header));
            prev_hash = hash;
            blocks.push(block);
        }
        let snap = make_snapshot(&chain);

        let n = run_for(SECS, || {
            let (mut storage, _dir) = tmp_storage();
            let mut cache = ScriptExecutionCache::new(1024);
            let mut bufs = ReplayBufs::new();
            for (i, block) in blocks.iter().enumerate() {
                let height = BlockHeight::new(u32::try_from(i + 1).expect("fits"));
                commit(
                    &mut bufs,
                    &mut storage,
                    black_box(block),
                    height,
                    &snap,
                    &mut cache,
                    i as u64,
                );
            }
            storage.shutdown().expect("shutdown");
        });
        eprintln!(
            "replay_chain_100:     {n} chains ({:.0} blocks/s)",
            n as f64 * f64::from(CHAIN_LEN) / SECS.as_secs_f64()
        );
    }
}
