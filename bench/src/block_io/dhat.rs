//! Block-IO heap profiler — dhat allocation analysis.
//!
//! Run with:
//!   cargo build --release --bin block-io-dhat
//!   ./target/release/block-io-dhat
//!
//! dhat writes `dhat-heap.json` in the current directory on exit.

#[path = "fixtures.rs"]
mod fixtures;

use fixtures::{
    make_block_data, make_header_bytes, seed_blocks, spawn_handle, tmp_dir, ITERS_MEDIUM,
    ITERS_SMALL,
};
use mantle::primitives::BlockHeight;
use std::hint::black_box;

#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

fn main() {
    let _profiler = dhat::Profiler::new_heap();

    scenario_write_sync();
    scenario_write_async();
    scenario_read_sequential();
    scenario_write_read_roundtrip();
    scenario_header_load();

    eprintln!("all scenarios complete — dhat summary follows");
}

fn scenario_write_sync() {
    let data = make_block_data(1_500_000);
    for _ in 0..ITERS_SMALL {
        let dir = tmp_dir("dhat-ws");
        let mut handle = spawn_handle(&dir);
        for h in 0..50u32 {
            handle
                .write_block(BlockHeight::new(h), data.clone())
                .expect("write_block");
        }
        handle.shutdown();
    }
    eprintln!("write_sync(50 x 1.5MB): {ITERS_SMALL} iters done");
}

fn scenario_write_async() {
    let data = make_block_data(1_500_000);
    for _ in 0..ITERS_SMALL {
        let dir = tmp_dir("dhat-wa");
        let mut handle = spawn_handle(&dir);
        for h in 0..50u32 {
            handle
                .write_block_async(BlockHeight::new(h), data.clone())
                .expect("write_block_async");
        }
        handle.shutdown();
    }
    eprintln!("write_async(50 x 1.5MB): {ITERS_SMALL} iters done");
}

fn scenario_read_sequential() {
    let data = make_block_data(1_500_000);
    let dir = tmp_dir("dhat-rs");
    let mut handle = spawn_handle(&dir);
    seed_blocks(&mut handle, 100, &data);
    handle.flush_block_index().expect("flush");

    for _ in 0..ITERS_SMALL {
        for h in 0..100u32 {
            black_box(handle.read_block(BlockHeight::new(h)).expect("read"));
        }
    }
    eprintln!("read_sequential(100 x 1.5MB): {ITERS_SMALL} iters done");

    handle.shutdown();
}

fn scenario_write_read_roundtrip() {
    let data = make_block_data(1_500_000);
    for _ in 0..ITERS_SMALL {
        let dir = tmp_dir("dhat-rt");
        let mut handle = spawn_handle(&dir);
        for h in 0..20u32 {
            handle
                .write_block(BlockHeight::new(h), data.clone())
                .expect("write");
        }
        for h in 0..20u32 {
            black_box(handle.read_block(BlockHeight::new(h)).expect("read"));
        }
        handle.shutdown();
    }
    eprintln!("write_read_roundtrip(20 x 1.5MB): {ITERS_SMALL} iters done");
}

fn scenario_header_load() {
    let hdr_data = make_header_bytes(10_000);
    let dir = tmp_dir("dhat-hdr");
    let mut handle = spawn_handle(&dir);
    handle
        .write_headers(BlockHeight::new(1), hdr_data)
        .expect("write_headers");

    for _ in 0..ITERS_MEDIUM {
        black_box(
            handle
                .load_cached_headers(BlockHeight::new(1))
                .expect("load"),
        );
    }
    eprintln!("header_load(10k): {ITERS_MEDIUM} iters done");

    handle.shutdown();
}
