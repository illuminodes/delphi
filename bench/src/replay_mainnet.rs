#![allow(clippy::cast_precision_loss)]
//! Replay-mainnet perf driver.
//!
//! Reads the fixtures written by `fetch-fixtures` and replays them through
//! the full validate+commit pipeline. Works for any height range including
//! mid-chain ranges (Ordinals, Runes era) where the UTXO set is not available:
//! in that case a dummy coin is returned for unknown inputs so throughput can
//! still be measured on real block data.
//!
//! Prerequisites:
//!   cargo run --release --bin fetch-fixtures [--preset ordinals|runes]
//!
//! Usage:
//!   cargo build --release --bin replay-mainnet
//!   ./target/release/replay-mainnet
//!   perf record -g ./target/release/replay-mainnet && perf report

use std::path::{Path, PathBuf};
use std::time::Instant;

use mantle::primitives::BlockHeight;
use mantle_consensus::script::ScriptExecutionCache;

#[path = "fixtures_common.rs"]
mod fixtures_common;
use fixtures_common::{
    load_block, load_headers, load_metadata, make_snapshot_from_genesis, make_snapshot_mid_chain,
    replay_block, tmp_storage,
};

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures")
}

fn main() {
    let fixtures = fixtures_dir();

    let start_height = load_metadata(&fixtures);
    let headers = load_headers(&fixtures);
    let n_blocks = u32::try_from(headers.len()).expect("fits");
    let end_height = start_height + n_blocks - 1;

    println!("Loaded {n_blocks} headers (heights {start_height}–{end_height}).");

    let mid_chain = start_height > 1;
    let snap = if mid_chain {
        make_snapshot_mid_chain(&headers, start_height)
    } else {
        make_snapshot_from_genesis(&headers)
    };
    if mid_chain {
        println!("Mid-chain mode: unknown UTXO inputs will use dummy coins (throughput only).");
    }

    print!("Loading {n_blocks} blocks into memory...");
    let blocks: Vec<_> = (start_height..=end_height)
        .map(|h| load_block(&fixtures, h))
        .collect();
    println!(" done.");

    let (mut storage, _tmp) = tmp_storage("replay-mainnet");
    let mut cache = ScriptExecutionCache::new(64 * 1024);

    println!("Replaying {n_blocks} blocks...");
    let t0 = Instant::now();

    for (i, block) in blocks.iter().enumerate() {
        let height = BlockHeight::new(start_height + u32::try_from(i).expect("fits"));
        let token = u64::try_from(i + 1).expect("fits");
        replay_block(
            block,
            height,
            &mut storage,
            &snap,
            &mut cache,
            token,
            mid_chain,
        );
    }

    let elapsed = t0.elapsed();
    let blocks_per_sec = f64::from(n_blocks) / elapsed.as_secs_f64();
    let ms_per_block = elapsed.as_secs_f64() * 1000.0 / f64::from(n_blocks);

    println!(
        "replay_mainnet_{start_height}_{end_height}: {elapsed:.3?}  \
         ({blocks_per_sec:.0} blocks/s,  {ms_per_block:.3} ms/block)"
    );

    storage.shutdown().expect("shutdown");
}
