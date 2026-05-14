//! `mantle-replay` — Stage 3 historical replay binary.
//!
//! Drives the replay loop committed to in `docs/architecture/replay.md`:
//!
//! ```text
//! for height in start..=target:
//!     hash  = headers[height]
//!     bytes = fetcher.read_block_bytes(height)
//!     block = Block::decode(bytes)
//!     validate_block_structure(&block)?
//!     validate_block_context(&block, &header_snapshot)?
//!     validator::validate_and_commit(
//!         &block, height, &mut storage, &header_snapshot,
//!         skip_script, Some(&mut exec_cache),
//!         if height % 2016 == 0 { Synced } else { Memory },
//!     )?
//! ```
//!
//! CLI: `mantle-replay [--data-dir DIR] [--network NAME] [--peer HOST:PORT]…
//!                    [--skip-script] [--assume-valid HEX]
//!                    [--start-height N] [--stop-height N] [--help]`.

mod iouring_fetcher;
mod report;

use std::net::SocketAddrV4;
use std::path::PathBuf;
use std::process::ExitCode;

use mantle::codec::Decode;
use mantle::collections::{BuildPrehashedHasher, PrehashedMap};
use mantle::network::Network;
use mantle::primitives::{Block, BlockHeader, BlockHeight, Hash256};
use mantle_consensus::script::ScriptExecutionCache;
use mantle_consensus::validation::{validate_block_context, validate_block_structure};
use mantle_storage::StorageHandle;
use mantle_storage::types::{Durability, ReqToken};
use mantle_validator::pipeline::validate_and_commit;
use mantle_validator::prefetched_view::{HeaderSnapshot, PrefetchedView};

use crate::iouring_fetcher::Fetcher;
use crate::report::Reporter;

struct ReplayCtx {
    storage: StorageHandle<'static>,
    skip_script: bool,
    allow_min_difficulty: bool,
    cache: ScriptExecutionCache,
    token: u64,
}

/// Block at which `Durability::Synced` flushes everything to disk. Matches
/// difficulty-adjustment-period boundaries — a natural checkpoint cadence.
const SYNC_EVERY: u32 = 2_016;
/// Cap script-execution cache at this many entries.
const SCRIPT_CACHE_ENTRIES: usize = 64 * 1024;

// ── CLI ──────────────────────────────────────────────────────────────────────

struct Args {
    data_dir: PathBuf,
    network: Network,
    extra_peers: Vec<SocketAddrV4>,
    skip_script: bool,
    /// Stop validating once `block.hash() == this` is committed (then exit OK).
    /// Phase 1 deliberately does *not* implement Bitcoin Core's "skip script
    /// for ancestors of assume-valid"; a future commit can wire that to
    /// `skip_script = true` for height ≤ found.
    #[allow(dead_code)]
    assume_valid: Option<Hash256>,
    /// First height to validate. Defaults to `storage.tip().height + 1`.
    start_height: Option<BlockHeight>,
    /// Last height to validate (inclusive). Defaults to whatever the peer
    /// reports as its current tip after a single `getheaders` page.
    stop_height: Option<BlockHeight>,
}

const HELP: &str = "\
mantle-replay — historical replay against a Bitcoin Core-compatible network

Usage:
    mantle-replay [OPTIONS]

Options:
    --data-dir DIR        Storage + block-cache root
                          (default: ./data/<network>)
    --network NAME        mainnet | testnet3 | testnet4 | signet | regtest
                          (default: mainnet). Aliases: main, test, testnet.
    --peer HOST:PORT      Extra P2P peer; may be repeated. Required when the
                          network has no built-in seed list (testnet3, signet,
                          regtest); optional otherwise (mainnet/testnet4).
    --peer-file PATH      Append one IP:PORT per line to the peer list.
                          Blank lines and `#`-prefixed comments are skipped.
                          Useful with the curated lists `discover-peers`
                          writes, or external sources like
                          https://testnet4.achownodes.xyz/seeds.txt.gz.
    --skip-script         Skip script verification (Stage 2; faster bring-up).
    --assume-valid HEX    Assume the chain leading to this block hash is
                          valid; runs script for descendants only.
                          (Phase 1: parsed but not yet honoured.)
    --start-height N      First height to validate. Default: storage tip + 1.
    --stop-height N       Last height to validate (inclusive).
                          Default: peer tip from one getheaders call.
    --help                Print this message and exit.
";

#[allow(clippy::too_many_lines)]
fn parse_args() -> Result<Args, String> {
    let mut data_dir: Option<PathBuf> = None;
    let mut network = Network::Mainnet;
    let mut extra_peers: Vec<SocketAddrV4> = Vec::new();
    let mut skip_script = false;
    let mut assume_valid: Option<Hash256> = None;
    let mut start_height: Option<BlockHeight> = None;
    let mut stop_height: Option<BlockHeight> = None;

    let raw: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < raw.len() {
        match raw[i].as_str() {
            "--help" | "-h" => {
                print!("{HELP}");
                std::process::exit(0);
            }
            "--data-dir" => {
                i += 1;
                let v = raw.get(i).ok_or("--data-dir requires a path")?;
                data_dir = Some(PathBuf::from(v));
            }
            "--network" => {
                i += 1;
                let v = raw.get(i).ok_or("--network requires a name")?;
                network = Network::parse(v).ok_or_else(|| format!("unknown network: {v}"))?;
            }
            "--peer" => {
                i += 1;
                let v = raw.get(i).ok_or("--peer requires HOST:PORT")?;
                let addr: SocketAddrV4 =
                    v.parse().map_err(|e| format!("invalid --peer {v}: {e}"))?;
                extra_peers.push(addr);
            }
            "--peer-file" => {
                i += 1;
                let v = raw.get(i).ok_or("--peer-file requires a path")?;
                let body =
                    std::fs::read_to_string(v).map_err(|e| format!("read --peer-file {v}: {e}"))?;
                let mut added = 0usize;
                for (lineno, line) in body.lines().enumerate() {
                    let trimmed = line.split('#').next().unwrap_or("").trim();
                    if trimmed.is_empty() {
                        continue;
                    }
                    match trimmed.parse::<SocketAddrV4>() {
                        Ok(addr) => {
                            extra_peers.push(addr);
                            added += 1;
                        }
                        Err(e) => {
                            return Err(format!(
                                "{v}:{}: invalid peer {trimmed:?}: {e}",
                                lineno + 1
                            ));
                        }
                    }
                }
                eprintln!("  loaded {added} peers from {v}");
            }
            "--skip-script" => {
                skip_script = true;
            }
            "--assume-valid" => {
                i += 1;
                let v = raw.get(i).ok_or("--assume-valid requires a hex hash")?;
                assume_valid = Some(
                    Hash256::from_display_hex(v)
                        .map_err(|e| format!("invalid --assume-valid {v}: {e}"))?,
                );
            }
            "--start-height" => {
                i += 1;
                let v = raw.get(i).ok_or("--start-height requires N")?;
                let n: u32 = v
                    .parse()
                    .map_err(|e| format!("invalid --start-height {v}: {e}"))?;
                start_height = Some(BlockHeight(n));
            }
            "--stop-height" => {
                i += 1;
                let v = raw.get(i).ok_or("--stop-height requires N")?;
                let n: u32 = v
                    .parse()
                    .map_err(|e| format!("invalid --stop-height {v}: {e}"))?;
                stop_height = Some(BlockHeight(n));
            }
            other => return Err(format!("unknown argument: {other}")),
        }
        i += 1;
    }

    let data_dir = data_dir.unwrap_or_else(|| PathBuf::from(format!("./data/{}", network.name())));

    Ok(Args {
        data_dir,
        network,
        extra_peers,
        skip_script,
        assume_valid,
        start_height,
        stop_height,
    })
}

// ── header snapshot construction ─────────────────────────────────────────────

/// Lookback applied to the snapshot when starting mid-chain. Big enough
/// to cover BIP113 MTP (11 ancestors) and one retarget interval (2 016
/// blocks back at retarget heights). Storage holds every committed
/// header, so this is just a contiguous range read.
const SNAPSHOT_LOOKBACK: u32 = 2_016;

/// Build a `HeaderSnapshot` for validating `[start..=stop]`. The snapshot
/// must carry enough *prior* headers that
/// `validate_block_context` can resolve the parent of `start`, compute
/// MTP (needs 11 ancestors), and look up the previous retarget boundary
/// (needs 2 016 ancestors at retarget heights). Sources:
///
/// - `headers`: the just-resolved headers for `[start..=stop]`.
/// - `storage` (when `start > 1`): the most recent
///   `SNAPSHOT_LOOKBACK` committed headers preceding `start`. Storage
///   exposes `read_header_at` keyed by height, so this is a tight loop.
/// - `network.genesis_header()` (when `start == 1`): a synthetic height-0
///   pre-genesis slot so block 1 has a parent for MTP. Earlier code
///   pushed `headers[0]` (block 1) into this slot, which made MTP at
///   height 1 == block 1's own time — failing BIP113's strict-`>` rule.
fn build_snapshot(
    start: BlockHeight,
    headers: &[BlockHeader],
    network: Network,
    storage: &mut StorageHandle,
) -> Result<HeaderSnapshot, String> {
    if headers.is_empty() {
        return Err("no headers to build a snapshot from".to_string());
    }
    if start.0 == 1 {
        let genesis = network.genesis_header().ok_or_else(|| {
            format!(
                "{} genesis header not yet baked in; supply --start-height \
                 along with a pre-populated headers.bin instead of starting \
                 from height 1",
                network.name()
            )
        })?;
        let mut entries: Vec<(Hash256, BlockHeader)> = Vec::with_capacity(headers.len() + 1);
        entries.push((genesis.hash(), genesis));
        for hdr in headers {
            entries.push((hdr.hash(), *hdr));
        }
        return HeaderSnapshot::new(BlockHeight(0), entries)
            .map_err(|e| format!("snapshot too large: {e:?}"));
    }
    // start > 1 — pull lookback from storage. Stored heights span
    // `[1..=tip]`, so we ask for `[max(1, start - SNAPSHOT_LOOKBACK)..=start - 1]`,
    // and prepend a synthetic height-0 slot carrying the network's genesis
    // header so that `expected_nbits` at the next retarget boundary
    // (which looks up `this_height - 2016`) finds a valid record at
    // height 0.
    let storage_lo = start.0.saturating_sub(SNAPSHOT_LOOKBACK).max(1);
    let need_genesis = start.0.saturating_sub(SNAPSHOT_LOOKBACK) == 0;
    let lookback_count = start.0 - storage_lo;
    let mut entries: Vec<(Hash256, BlockHeader)> = Vec::with_capacity(
        usize::from(need_genesis) + usize::try_from(lookback_count).unwrap_or(0) + headers.len(),
    );
    let base = if need_genesis {
        let genesis = network.genesis_header().ok_or_else(|| {
            format!(
                "{} genesis header not yet baked in; cannot build a \
                 snapshot whose lookback reaches height 0",
                network.name()
            )
        })?;
        entries.push((genesis.hash(), genesis));
        BlockHeight(0)
    } else {
        BlockHeight(storage_lo)
    };
    for h in storage_lo..start.0 {
        let hdr = storage
            .read_header_at(BlockHeight(h))
            .map_err(|e| format!("read stored header at {h}: {e}"))?
            .ok_or_else(|| format!("storage missing committed header at height {h}"))?;
        entries.push((hdr.hash(), hdr));
    }
    for hdr in headers {
        entries.push((hdr.hash(), *hdr));
    }
    HeaderSnapshot::new(base, entries).map_err(|e| format!("snapshot too large: {e:?}"))
}

// ── main ─────────────────────────────────────────────────────────────────────

fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("error: {e}");
            eprintln!("{HELP}");
            return ExitCode::from(2);
        }
    };

    if let Err(e) = std::fs::create_dir_all(&args.data_dir) {
        eprintln!(
            "error: cannot create data dir {}: {e}",
            args.data_dir.display()
        );
        return ExitCode::FAILURE;
    }

    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("fatal: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &Args) -> Result<(), String> {
    eprintln!(
        "mantle-replay starting on {} (data-dir {})",
        args.network.name(),
        args.data_dir.display()
    );

    // ── Storage ──────────────────────────────────────────────────────────────
    let mut storage =
        StorageHandle::open_robin_hood(&args.data_dir).map_err(|e| format!("open storage: {e}"))?;
    let tip = storage.tip();
    let next_to_validate = BlockHeight(tip.height.0.saturating_add(1));
    let start_height = args.start_height.unwrap_or(next_to_validate);
    if start_height < next_to_validate {
        return Err(format!(
            "--start-height {} precedes storage tip + 1 ({}); refusing to overwrite",
            start_height.0, next_to_validate.0
        ));
    }
    eprintln!(
        "  storage tip = {} ({:?}); validating from height {}",
        tip.height.0, tip.hash, start_height.0,
    );

    // ── Peers ────────────────────────────────────────────────────────────────
    // Peer list comes exclusively from `--peer` / `--peer-file`. DNS seeds
    // were unreliable in practice (stale entries that aggressively close on
    // version handshake, polluting the pool) — the curated lists from
    // `rank-peers` give us live peers without the seed-induced flakiness.
    let peer_addrs: Vec<SocketAddrV4> = args.extra_peers.clone();
    if peer_addrs.is_empty() {
        return Err(format!(
            "no peers: supply --peer HOST:PORT or --peer-file PATH (network={})",
            args.network.name()
        ));
    }
    let mut fetcher = Fetcher::new(args.network, peer_addrs, &args.data_dir)
        .map_err(|e| format!("fetcher init: {e}"))?;

    // ── Phase 1: sync all headers to tip ──────────────────────────────────
    let locator = if start_height.0 <= 1 {
        args.network.genesis_hash()
    } else {
        tip.hash
    };
    let stop_height = args.stop_height.unwrap_or(BlockHeight(u32::MAX));

    eprintln!(
        "  syncing headers from height {} to {}…",
        start_height.0,
        if stop_height.0 == u32::MAX {
            "tip".to_string()
        } else {
            stop_height.0.to_string()
        },
    );

    let header_pairs = fetcher
        .sync_headers(start_height, stop_height, locator)
        .map_err(|e| format!("sync headers: {e:?}"))?;

    if header_pairs.is_empty() {
        eprintln!("  no new headers from peers — already at tip");
        storage.shutdown().map_err(|e| format!("shutdown: {e}"))?;
        return Ok(());
    }

    let actual_stop = header_pairs.last().unwrap().0;
    eprintln!(
        "  got {} headers ({}..={})",
        header_pairs.len(),
        start_height.0,
        actual_stop.0,
    );

    // ── Phase 2: download all blocks ────────────────────────────────────
    eprintln!("  downloading blocks…");
    fetcher
        .fetch_blocks(&header_pairs)
        .map_err(|e| format!("fetch blocks: {e:?}"))?;

    // ── Phase 3: replay ─────────────────────────────────────────────────
    let headers: Vec<BlockHeader> = header_pairs.iter().map(|(_, h)| *h).collect();
    let snapshot = build_snapshot(start_height, &headers, args.network, &mut storage)?;

    let allow_min_difficulty = matches!(args.network, Network::Testnet3 | Network::Testnet4);
    let mut ctx = ReplayCtx {
        storage,
        skip_script: args.skip_script,
        allow_min_difficulty,
        cache: ScriptExecutionCache::new(SCRIPT_CACHE_ENTRIES),
        token: u64::from(tip.height.0).saturating_add(1),
    };

    eprintln!(
        "  replaying {}..={} ({} script verification)…",
        start_height.0,
        actual_stop.0,
        if args.skip_script { "skipping" } else { "with" },
    );

    let mut reporter = Reporter::new(start_height, actual_stop);
    let mut last_committed = tip.height;

    for (height, header) in &header_pairs {
        replay_one_block(*height, *header, &mut fetcher, &snapshot, &mut ctx)?;
        last_committed = *height;
        reporter.tick(*height);
    }
    reporter.done(last_committed);

    eprintln!("IBD complete at height {}", last_committed.0);
    ctx.storage
        .shutdown()
        .map_err(|e| format!("shutdown storage: {e}"))?;
    Ok(())
}

fn replay_one_block(
    height: BlockHeight,
    header: BlockHeader,
    fetcher: &mut Fetcher,
    snapshot: &HeaderSnapshot,
    ctx: &mut ReplayCtx,
) -> Result<(), String> {
    let bytes = fetcher
        .read_block_bytes(height)
        .map_err(|e| format!("read block {} bytes: {e}", height.0))?;

    let block = Block::decode(&mut std::io::Cursor::new(&bytes))
        .map_err(|e| format!("decode block {}: {e}", height.0))?;

    if block.header.hash() != header.hash() {
        return Err(format!(
            "block {} hash mismatch: header says {}, payload says {}",
            height.0,
            header.hash().to_display_hex(),
            block.header.hash().to_display_hex()
        ));
    }

    validate_block_structure(&block)
        .map_err(|e| format!("structure check at {}: {e:?}", height.0))?;

    let empty_coins = PrehashedMap::with_hasher(BuildPrehashedHasher);
    let context_view = PrefetchedView::new(&empty_coins, snapshot, ctx.storage.tip());
    validate_block_context(&block, &context_view, ctx.allow_min_difficulty)
        .map_err(|e| format!("context check at {}: {e:?}", height.0))?;

    let durability = if height.0.is_multiple_of(SYNC_EVERY) {
        Durability::Synced
    } else {
        Durability::Memory
    };

    validate_and_commit(
        &block,
        height,
        &mut ctx.storage,
        snapshot,
        ctx.skip_script,
        Some(&mut ctx.cache),
        durability,
        ReqToken(ctx.token),
    )
    .map_err(|e| format!("validate_and_commit at {}: {e}", height.0))?;

    ctx.token = ctx.token.saturating_add(1);
    Ok(())
}
