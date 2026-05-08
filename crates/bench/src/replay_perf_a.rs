//! Replay perf — workload A only: coinbase-only block (no UTXO lookups).
//!
//! Fresh storage each iter to avoid BIP30 on repeated coinbase txid.
#![allow(clippy::cast_precision_loss)]

use std::hint::black_box;

use mantle::primitives::{BlockHeight, Hash256};
use mantle_consensus::script::ScriptExecutionCache;

#[path = "replay_perf_common.rs"]
mod common;
use common::{SECS, coinbase_tx, commit, make_block, make_snapshot, run_for, tmp_storage};

fn main() {
    eprintln!("[A] start");
    let block = make_block(Hash256::ZERO, 1, vec![coinbase_tx(1, 50_0000_0000)]);
    let hash = block.header.hash();
    let snap = make_snapshot(&[(hash, block.header)]);

    let mut iter_idx = 0u32;
    let n = run_for(SECS, || {
        eprint!("[A] {iter_idx} ");
        let (mut storage, _dir) = tmp_storage();
        let mut cache = ScriptExecutionCache::new(1024);
        eprint!("C ");
        commit(
            &mut storage,
            black_box(&block),
            BlockHeight(1),
            &snap,
            &mut cache,
            1,
        );
        eprint!("X ");
        storage.shutdown().expect("shutdown");
        eprintln!("ok");
        iter_idx += 1;
    });
    eprintln!(
        "replay_coinbase_only: {n} iters  ({:.0} blocks/s)",
        n as f64 / SECS.as_secs_f64()
    );
}
