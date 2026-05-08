//! Replay perf — workload C only: 100-block chain, sequential.
#![allow(clippy::cast_precision_loss)]

use std::hint::black_box;

use mantle::primitives::{Block, BlockHeader, BlockHeight, Hash256};
use mantle_consensus::script::ScriptExecutionCache;

#[path = "replay_perf_common.rs"]
mod common;
use common::{coinbase_tx, commit, make_block, make_snapshot, run_for, tmp_storage, SECS};

fn main() {
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

    let mut iter_idx = 0u32;
    let n = run_for(SECS, || {
        eprint!("[C] iter={iter_idx} ");
        let (mut storage, _dir) = tmp_storage();
        let mut cache = ScriptExecutionCache::new(1024);
        for (i, block) in blocks.iter().enumerate() {
            if i % 10 == 0 { eprint!("h{} ", i + 1); }
            let height = BlockHeight(u32::try_from(i + 1).expect("fits"));
            commit(&mut storage, black_box(block), height, &snap, &mut cache, i as u64);
        }
        eprint!("X ");
        storage.shutdown().expect("shutdown");
        eprintln!("done");
        iter_idx += 1;
    });
    eprintln!("replay_chain_100:     {n} chains ({:.0} blocks/s)",
        n as f64 * f64::from(CHAIN_LEN) / SECS.as_secs_f64());
}
