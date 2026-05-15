//! dhat heap profiler for the replay path.
//!
//! Run with:
//!   cargo build --release --bin replay-dhat
//!   ./target/release/replay-dhat
//!
//! dhat writes `dhat-heap.json` in the current directory on exit.
//! Open it at <https://nnethercote.github.io/dh_view/dh_view.html>.

use mantle::primitives::{
    Block, BlockHeader, BlockHeight, Hash256, Locktime, OutPoint, Satoshi, Sequence, Transaction,
    TxHash, TxIn, TxOut, TxVersion, Witness,
};
use mantle_consensus::script::ScriptExecutionCache;

#[path = "replay_perf_common.rs"]
mod common;
use common::{coinbase_tx, commit, make_block, make_snapshot, spend_tx, tmp_storage, ReplayBufs};

#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

fn main() {
    let _profiler = dhat::Profiler::new_heap();
    scenario_coinbase_only();
    scenario_spend_blocks();
}

fn scenario_coinbase_only() {
    let (mut storage, _dir) = tmp_storage();
    let mut bufs = ReplayBufs::new();
    let mut cache = ScriptExecutionCache::new(1024);
    let mut chain: Vec<(Hash256, BlockHeader)> = Vec::new();
    let mut prev_hash = Hash256::ZERO;
    for i in 1u32..=200 {
        let block = make_block(prev_hash, i, vec![coinbase_tx(i, 50_0000_0000)]);
        let hash = block.header.hash();
        chain.push((hash, block.header));
        let snap = make_snapshot(&chain);
        commit(
            &mut bufs,
            &mut storage,
            &block,
            BlockHeight::new(i),
            &snap,
            &mut cache,
            u64::from(i),
        );
        prev_hash = hash;
    }
    storage.shutdown().expect("shutdown");
}

fn scenario_spend_blocks() {
    const SPENDS: usize = 20;
    const ROUNDS: usize = 20;
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
        outputs: (0..SPENDS)
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
        .chain((0..SPENDS).map(|i| spend_tx(fund_txid, u32::try_from(i).expect("fits"), 999)))
        .collect();
    let spend_block = make_block(prev_hash, SPEND_H, spend_txs);
    let spend_hash = spend_block.header.hash();

    let mut chain = vec![(fund_hash, fund_block.header)];
    for b in &filler {
        chain.push((b.header.hash(), b.header));
    }
    chain.push((spend_hash, spend_block.header));
    let snap = make_snapshot(&chain);

    for _ in 0..ROUNDS {
        let (mut storage, _dir) = tmp_storage();
        let mut bufs = ReplayBufs::new();
        let mut cache = ScriptExecutionCache::new(1024);
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
            commit(
                &mut bufs,
                &mut storage,
                b,
                BlockHeight::new(2 + u32::try_from(i).expect("fits")),
                &snap,
                &mut cache,
                2 + u64::try_from(i).expect("fits"),
            );
        }
        commit(
            &mut bufs,
            &mut storage,
            &spend_block,
            BlockHeight::new(SPEND_H),
            &snap,
            &mut cache,
            u64::from(SPEND_H),
        );
        storage.shutdown().expect("shutdown");
    }
}
