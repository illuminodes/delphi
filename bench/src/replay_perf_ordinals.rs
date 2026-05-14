//! Replay perf — ordinals fixture blocks (heights 767430–768430).
//!
//! Loads real mainnet blocks from fixtures/ordinals/ and replays them
//! through the full validate+commit pipeline for 5 s, reporting throughput.
//!
//! Prerequisites:
//!   cargo run --release --bin fetch-fixtures -- --preset ordinals
//!
//! Usage:
//!   cargo run --release --bin replay-perf-ordinals
#![allow(clippy::cast_precision_loss)]

use std::time::{Duration, Instant};

use mantle::primitives::BlockHeight;
use mantle_consensus::script::ScriptExecutionCache;

#[path = "fixtures_common.rs"]
mod fixtures_common;
use fixtures_common::{
    ORDINALS_SAMPLE_OFFSETS, fixtures_dir, load_block_bytes, load_headers, load_metadata,
    make_snapshot_mid_chain, replay_block_ref, tmp_storage,
};

const SECS: Duration = Duration::from_secs(5);

fn main() {
    let dir = fixtures_dir("ordinals");
    let start = load_metadata(&dir);
    let headers = load_headers(&dir);
    let snap = make_snapshot_mid_chain(&headers, start);

    // Load only the representative sample blocks, not all 1001.
    let samples: Vec<(u32, Vec<u8>)> = ORDINALS_SAMPLE_OFFSETS
        .iter()
        .map(|&off| {
            let h = start + off;
            let wire = load_block_bytes(&dir, h);
            (h, wire)
        })
        .collect();

    eprintln!(
        "[ordinals] {} sample blocks loaded (heights: {})",
        samples.len(),
        samples
            .iter()
            .map(|(h, _)| h.to_string())
            .collect::<Vec<_>>()
            .join(", ")
    );
    eprintln!("[ordinals] running for {}s", SECS.as_secs());

    let deadline = Instant::now() + SECS;
    let mut total_blocks = 0u64;
    let mut token = 1u64;
    let mut arena: Vec<u8> = Vec::new();

    'outer: loop {
        for (height, wire) in &samples {
            if Instant::now() >= deadline {
                break 'outer;
            }
            // Fresh storage each block — avoids UTXO state accumulating across
            // unrelated sample blocks and keeps I/O cost comparable across iters.
            let (mut storage, _tmp) = tmp_storage("replay-perf-ordinals");
            let mut cache = ScriptExecutionCache::new(64 * 1024);
            replay_block_ref(
                wire,
                BlockHeight(*height),
                &mut storage,
                &snap,
                &mut cache,
                token,
                true,
                &mut arena,
            );
            storage.shutdown().expect("shutdown");
            total_blocks += 1;
            token += 1;
        }
    }

    let elapsed = SECS.as_secs_f64();
    eprintln!(
        "replay_ordinals_samples: {total_blocks} blocks  ({:.1} blocks/s,  {:.1} ms/block)",
        total_blocks as f64 / elapsed,
        elapsed * 1000.0 / total_blocks as f64,
    );
}
