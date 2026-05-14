//! dhat heap profiler — ordinals fixture blocks (heights 767430–768430).
//!
//! Prerequisites:
//!   cargo run --release --bin fetch-fixtures -- --preset ordinals
//!
//! Usage:
//!   cargo build --release --bin replay-dhat-ordinals
//!   ./target/release/replay-dhat-ordinals
//!
//! dhat writes `dhat-heap.json` in the current directory on exit.
//! Open it at <https://nnethercote.github.io/dh_view/dh_view.html>.

use mantle::primitives::BlockHeight;
use mantle_consensus::script::ScriptExecutionCache;

#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

#[path = "fixtures_common.rs"]
mod fixtures_common;
use fixtures_common::{
    ORDINALS_SAMPLE_OFFSETS, fixtures_dir, load_block_bytes, load_headers, load_metadata,
    make_snapshot_mid_chain, replay_block_ref, tmp_storage,
};

fn main() {
    let _profiler = dhat::Profiler::new_heap();

    let dir = fixtures_dir("ordinals");
    let start = load_metadata(&dir);
    let headers = load_headers(&dir);
    let snap = make_snapshot_mid_chain(&headers, start);

    let samples: Vec<(u32, Vec<u8>)> = ORDINALS_SAMPLE_OFFSETS
        .iter()
        .map(|&off| (start + off, load_block_bytes(&dir, start + off)))
        .collect();

    eprintln!(
        "dhat-ordinals: replaying {} sample blocks (heights: {})",
        samples.len(),
        samples
            .iter()
            .map(|(h, _)| h.to_string())
            .collect::<Vec<_>>()
            .join(", ")
    );

    let mut arena: Vec<u8> = Vec::new();
    for (token, (height, wire)) in samples.iter().enumerate() {
        let (mut storage, _tmp) = tmp_storage("replay-dhat-ordinals");
        let mut cache = ScriptExecutionCache::new(64 * 1024);
        replay_block_ref(
            wire,
            BlockHeight::new(*height),
            &mut storage,
            &snap,
            &mut cache,
            u64::try_from(token + 1).expect("fits"),
            true,
            &mut arena,
        );
        storage.shutdown().expect("shutdown");
    }

    eprintln!("dhat-ordinals: done");
}
