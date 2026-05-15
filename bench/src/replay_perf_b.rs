//! Replay perf — workload B only: block with 50 UTXO spends.
//!
//! Setup (once, outside the timed loop):
//!   Builds a seed storage directory with the full 102-block setup chain
//!   (fund coinbase at height 1, 100 filler blocks for maturity).
//!
//! Timed region (per iteration):
//!   Copy seed dir → open storage → commit the spend block → shutdown → delete.
//!   This isolates the cost of the spend block commit itself.
#![allow(clippy::cast_precision_loss)]

use std::hint::black_box;
use std::time::Instant;

use mantle::primitives::{
    BlockHeight, Hash256, Locktime, OutPoint, Satoshi, Sequence, Transaction, TxHash, TxIn, TxOut,
    TxVersion, Witness,
};
use mantle_consensus::script::ScriptExecutionCache;

#[path = "replay_perf_common.rs"]
mod common;
use common::{
    SECS, TmpDir, coinbase_tx, commit, make_block, make_snapshot, run_for, spend_tx, tmp_storage,
};

fn copy_dir(src: &std::path::Path, dst: &std::path::Path) {
    std::fs::create_dir_all(dst).expect("create dst dir");
    for entry in std::fs::read_dir(src).expect("read src dir") {
        let entry = entry.expect("dir entry");
        let dst_path = dst.join(entry.file_name());
        std::fs::copy(entry.path(), dst_path).expect("copy file");
    }
}

const N_SPENDS: usize = 50;
const MATURITY: u32 = 100;
const FUND_H: u32 = 1;
const SPEND_H: u32 = FUND_H + MATURITY + 1;

struct Scenario {
    seed_dir: TmpDir,
    spend_block: mantle::primitives::Block,
    snap: mantle_validator::prefetched_view::HeaderSnapshot,
}

fn build_scenario() -> Scenario {
    use mantle::primitives::{Block, BlockHeader};

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

    let mut chain: Vec<(Hash256, BlockHeader)> = vec![(fund_hash, fund_block.header)];
    for b in &filler {
        chain.push((b.header.hash(), b.header));
    }
    chain.push((spend_hash, spend_block.header));
    let snap = make_snapshot(&chain);

    eprintln!("[B] building seed storage...");
    let (mut seed_storage, seed_dir) = tmp_storage();
    let mut cache = ScriptExecutionCache::new(1024);
    commit(
        &mut seed_storage,
        &fund_block,
        BlockHeight::new(FUND_H),
        &snap,
        &mut cache,
        1,
    );
    for (i, b) in filler.iter().enumerate() {
        commit(
            &mut seed_storage,
            b,
            BlockHeight::new(2 + u32::try_from(i).expect("fits")),
            &snap,
            &mut cache,
            2 + u64::try_from(i).expect("fits"),
        );
    }
    seed_storage.shutdown().expect("seed shutdown");
    eprintln!("[B] seed ready at {}", seed_dir.0.display());

    Scenario {
        seed_dir,
        spend_block,
        snap,
    }
}

fn main() {
    let Scenario {
        seed_dir,
        spend_block,
        snap,
    } = build_scenario();

    let mut iter_idx = 0u32;
    let workload_start = Instant::now();
    let n = run_for(SECS, || {
        let it_start = Instant::now();
        let ns = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .subsec_nanos();
        let iter_path = std::env::temp_dir().join(format!(
            "mantle-replay-perf-b-iter-{}-{}",
            std::process::id(),
            ns
        ));
        copy_dir(&seed_dir.0, &iter_path);
        let _iter_dir = TmpDir(iter_path.clone());

        let mut storage = mantle_storage::StorageHandle::open(&iter_path).expect("open");
        let mut cache = ScriptExecutionCache::new(1024);
        commit(
            &mut storage,
            black_box(&spend_block),
            BlockHeight::new(SPEND_H),
            &snap,
            &mut cache,
            u64::from(SPEND_H),
        );
        storage.shutdown().expect("shutdown");

        let dt = it_start.elapsed();
        if iter_idx < 5 || iter_idx.is_multiple_of(50) {
            eprintln!(
                "[B] iter={iter_idx} dt={:.2}ms wall={:.2}s",
                dt.as_secs_f64() * 1000.0,
                workload_start.elapsed().as_secs_f64()
            );
        }
        iter_idx += 1;
    });
    eprintln!(
        "[B] done wall={:.2}s n={n}",
        workload_start.elapsed().as_secs_f64()
    );
    eprintln!(
        "replay_50_spends:     {n} iters  ({:.0} spend-blocks/s)",
        n as f64 / SECS.as_secs_f64()
    );
}
