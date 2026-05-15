//! Transport CPU profiler — pprof-rs sampling analysis.
//!
//! Run with:
//!   cargo build --release --bin transport-pprof
//!   ./target/release/transport-pprof [SCENARIO]
//!
//! Without arguments, runs all scenarios. Pass a scenario name to run just one:
//!   ./target/release/transport-pprof `arena_frame_256b`
//!   ./target/release/transport-pprof `cqe_roundtrip`

#[path = "fixtures.rs"]
mod fixtures;

use fixtures::{make_pool_half_full, mainnet_magic};
use mantle::network::Command;
use mantle_transport::{CqeData, Op, SendArena};
use std::hint::black_box;
use std::time::{Duration, Instant};

fn print_top(report: &pprof::Report, name: &str, iters: u64, duration_secs: u64) {
    println!("── {name} ({iters} iters, {duration_secs}s) ──");

    let total_samples: isize = report.data.values().sum();
    if total_samples == 0 {
        println!("  (no samples captured)");
        println!();
        return;
    }

    let mut flat: std::collections::HashMap<String, isize> = std::collections::HashMap::new();
    for (frames, &count) in &report.data {
        let mut seen = std::collections::HashSet::new();
        for sym_list in &frames.frames {
            for sym in sym_list {
                let fname = sym.name();
                if seen.insert(fname.clone()) {
                    *flat.entry(fname).or_default() += count;
                }
            }
        }
    }

    let mut entries: Vec<_> = flat.into_iter().collect();
    entries.sort_by_key(|e| std::cmp::Reverse(e.1));

    println!("  {:<70} {:>8} {:>7}", "FUNCTION", "SAMPLES", "%");
    for (fname, count) in entries.iter().take(25) {
        #[allow(clippy::cast_precision_loss)]
        let pct = *count as f64 / total_samples as f64 * 100.0;
        println!("  {fname:<70} {count:>8} {pct:>6.1}%");
    }
    println!();
}

fn profile<F: FnMut()>(name: &str, duration_secs: u64, mut workload: F) {
    let guard = pprof::ProfilerGuardBuilder::default()
        .frequency(997)
        .build()
        .expect("pprof profiler must initialize");

    let deadline = Instant::now() + Duration::from_secs(duration_secs);
    let mut iters = 0u64;
    while Instant::now() < deadline {
        workload();
        iters += 1;
    }

    let report = guard.report().build().expect("pprof report must build");
    print_top(&report, name, iters, duration_secs);
}

fn run_cqe_roundtrip() {
    let mut slot = 0u16;
    profile("cqe_roundtrip", 3, || {
        let raw = CqeData::encode(black_box(slot), black_box(7), Op::Recv);
        black_box(CqeData::decode(raw));
        slot = slot.wrapping_add(1);
    });
}

fn run_cqe_send_roundtrip() {
    let mut slot = 0u16;
    profile("cqe_send_roundtrip", 3, || {
        let raw = CqeData::encode_send(black_box(slot), black_box(7), black_box(slot));
        black_box(CqeData::decode_send_seq(raw));
        slot = slot.wrapping_add(1);
    });
}

fn run_arena_alloc_release() {
    let mut arena = SendArena::new();
    let magic = mainnet_magic();
    profile("arena_alloc_release", 3, || {
        let seq = arena
            .frame_into(magic, Command::Verack, black_box(&[]))
            .expect("should allocate");
        arena.release(seq);
    });
}

fn run_arena_frame_256b() {
    let mut arena = SendArena::new();
    let magic = mainnet_magic();
    let payload = vec![0xab_u8; 256];
    profile("arena_frame_256b", 3, || {
        let seq = arena
            .frame_into(magic, Command::Inv, black_box(&payload))
            .expect("should allocate");
        arena.release(seq);
    });
}

fn run_arena_burst_64() {
    let mut arena = SendArena::new();
    let magic = mainnet_magic();
    let mut seqs = Vec::with_capacity(64);
    profile("arena_burst_64", 3, || {
        seqs.clear();
        for _ in 0..64 {
            if let Some(seq) = arena.frame_into(magic, Command::Verack, &[]) {
                seqs.push(seq);
            }
        }
        for &seq in &seqs {
            arena.release(seq);
        }
    });
}

fn run_pool_iter_mut() {
    let mut pool = make_pool_half_full();
    profile("pool_iter_mut", 3, || {
        for (_slot, peer) in pool.iter_mut() {
            black_box(&peer.state);
        }
    });
}

fn run_pool_dead_slots() {
    let mut pool = make_pool_half_full();
    profile("pool_dead_slots", 3, || {
        black_box(pool.dead_slots());
    });
}

fn run_encode_version() {
    let mut buf = Vec::new();
    profile("encode_version", 3, || {
        mantle_transport::encode_version(black_box("mantle:0.1.0"), &mut buf);
        black_box(buf.len());
    });
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let filter = args.get(1).map(String::as_str);

    let scenarios: &[(&str, fn())] = &[
        ("cqe_roundtrip", run_cqe_roundtrip),
        ("cqe_send_roundtrip", run_cqe_send_roundtrip),
        ("arena_alloc_release", run_arena_alloc_release),
        ("arena_frame_256b", run_arena_frame_256b),
        ("arena_burst_64", run_arena_burst_64),
        ("pool_iter_mut", run_pool_iter_mut),
        ("pool_dead_slots", run_pool_dead_slots),
        ("encode_version", run_encode_version),
    ];

    let mut ran = 0;
    for &(name, f) in scenarios {
        if filter.is_none() || filter == Some(name) {
            f();
            ran += 1;
        }
    }

    if ran == 0 {
        eprintln!("no matching scenario. available:");
        for &(name, _) in scenarios {
            eprintln!("  {name}");
        }
        std::process::exit(1);
    }
}
