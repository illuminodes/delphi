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

mod fetcher;
mod iouring_fetcher;
mod report;

use std::net::{SocketAddr, SocketAddrV4, ToSocketAddrs};
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

use crate::fetcher::Fetcher;
use crate::iouring_fetcher::Fetcher as IoUringFetcher;
use crate::report::Reporter;

enum AnyFetcher {
    Legacy(Fetcher),
    IoUring(IoUringFetcher),
}

impl AnyFetcher {
    fn sync_headers(
        &mut self,
        start: BlockHeight,
        end: BlockHeight,
        locator: Hash256,
    ) -> Result<Vec<(BlockHeight, mantle::primitives::BlockHeader)>, String> {
        match self {
            Self::Legacy(f) => f.sync_headers(start, end, locator).map_err(|e| format!("fetch: {e}")),
            Self::IoUring(f) => f.sync_headers(start, end, locator).map_err(|e| format!("fetch: {e}")),
        }
    }

    fn fetch_blocks(&mut self, headers: &[(BlockHeight, mantle::primitives::BlockHeader)]) -> Result<(), String> {
        match self {
            Self::Legacy(f) => f.fetch_blocks(headers).map_err(|e| format!("fetch: {e}")),
            Self::IoUring(f) => f.fetch_blocks(headers).map_err(|e| format!("fetch: {e}")),
        }
    }

    fn read_block_bytes(&self, height: BlockHeight) -> Result<Vec<u8>, String> {
        match self {
            Self::Legacy(f) => f.read_block_bytes(height).map_err(|e| format!("read block {} from disk: {e}", height.0)),
            Self::IoUring(f) => f.read_block_bytes(height).map_err(|e| format!("read block {} from disk: {e}", height.0)),
        }
    }
}

type HeaderPairs = Vec<(BlockHeight, BlockHeader)>;
type ResolvedRange = (BlockHeight, BlockHeight, HeaderPairs);

struct ReplayCtx {
    storage: StorageHandle,
    skip_script: bool,
    cache: ScriptExecutionCache,
    token: u64,
}

/// Headers per `getheaders` response. Used to chunk header sync.
const HEADERS_PER_PAGE: u32 = 2_000;
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
    use_iouring: bool,
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
    --data-dir DIR        Storage + block-cache root (default: ./mantle-data)
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
    --use-iouring         Use the io_uring fetcher instead of the legacy fetcher.
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
    let mut data_dir = PathBuf::from("./mantle-data");
    let mut network = Network::Mainnet;
    let mut extra_peers: Vec<SocketAddrV4> = Vec::new();
    let mut use_iouring = false;
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
                data_dir = PathBuf::from(v);
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
                let body = std::fs::read_to_string(v)
                    .map_err(|e| format!("read --peer-file {v}: {e}"))?;
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
            "--use-iouring" => {
                use_iouring = true;
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

    Ok(Args {
        data_dir,
        network,
        extra_peers,
        use_iouring,
        skip_script,
        assume_valid,
        start_height,
        stop_height,
    })
}

// ── headers cache I/O ────────────────────────────────────────────────────────

/// Read a previously-persisted `chain_headers.bin` (raw 80-byte records).
/// Returns `(start_height, headers)` if the file and its sidecar
/// `metadata.bin` are both present; otherwise `Ok(None)`.
///
/// The fetcher's headers cache is named `chain_headers.bin` to avoid
/// colliding with `<data-dir>/headers.bin`, which the storage engine owns
/// and writes in a different (112-byte-per-record) format.
fn try_load_headers(
    data_dir: &std::path::Path,
) -> std::io::Result<Option<(BlockHeight, Vec<BlockHeader>)>> {
    let headers_path = data_dir.join("chain_headers.bin");
    let metadata_path = data_dir.join("metadata.bin");
    if !headers_path.exists() || !metadata_path.exists() {
        return Ok(None);
    }
    let metadata = std::fs::read(&metadata_path)?;
    if metadata.len() != 4 {
        return Ok(None);
    }
    let start = u32::from_le_bytes(metadata.as_slice().try_into().expect("len == 4"));

    let bytes = std::fs::read(&headers_path)?;
    if !bytes.len().is_multiple_of(80) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "chain_headers.bin size is not a multiple of 80",
        ));
    }
    let mut headers = Vec::with_capacity(bytes.len() / 80);
    for chunk in bytes.chunks_exact(80) {
        let arr: &[u8; 80] = chunk.try_into().expect("chunk is 80 bytes");
        headers.push(
            BlockHeader::from_bytes(arr)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?,
        );
    }
    Ok(Some((BlockHeight(start), headers)))
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
        usize::from(need_genesis)
            + usize::try_from(lookback_count).unwrap_or(0)
            + headers.len(),
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
        StorageHandle::open(&args.data_dir).map_err(|e| format!("open storage: {e}"))?;
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
    // Seeds may be either literal `IP:PORT` (mainnet's hand-picked list) or
    // DNS `HOST:PORT` (e.g. testnet4 / signet). Try literal parse first; on
    // failure, fall back to a system DNS lookup and take any v4 results.
    let mut peer_addrs: Vec<SocketAddrV4> = args.extra_peers.clone();
    for seed in args.network.default_seeds() {
        if let Ok(addr) = seed.parse::<SocketAddrV4>() {
            peer_addrs.push(addr);
            continue;
        }
        match seed.to_socket_addrs() {
            Ok(iter) => {
                let before = peer_addrs.len();
                for addr in iter {
                    if let SocketAddr::V4(v4) = addr {
                        peer_addrs.push(v4);
                    }
                }
                let resolved = peer_addrs.len() - before;
                if resolved == 0 {
                    eprintln!("  seed {seed} resolved to 0 IPv4 addresses (skipped)");
                } else {
                    eprintln!("  seed {seed} resolved to {resolved} address(es)");
                }
            }
            Err(e) => eprintln!("  failed to resolve seed {seed}: {e}"),
        }
    }
    if peer_addrs.is_empty() {
        return Err(format!(
            "no peers: {} has no reachable seeds; supply --peer HOST:PORT",
            args.network.name()
        ));
    }
    let mut fetcher: AnyFetcher = if args.use_iouring {
        eprintln!("  using io_uring fetcher");
        AnyFetcher::IoUring(
            IoUringFetcher::new(args.network, peer_addrs, args.data_dir.clone())
                .map_err(|e| format!("fetcher init: {e}"))?,
        )
    } else {
        AnyFetcher::Legacy(
            Fetcher::new(args.network, peer_addrs, args.data_dir.clone())
                .map_err(|e| format!("fetcher init: {e}"))?,
        )
    };

    // ── Headers + blocks ─────────────────────────────────────────────────────
    let (start_height, stop_height, header_pairs) =
        resolve_headers_and_blocks(args, start_height, tip.hash, &mut fetcher)?;

    // ── Header snapshot for context validation ──────────────────────────────
    let headers: Vec<BlockHeader> = header_pairs.iter().map(|(_, h)| *h).collect();
    let snapshot = build_snapshot(start_height, &headers, args.network, &mut storage)?;

    // ── Validation loop ──────────────────────────────────────────────────────
    eprintln!(
        "  replaying ({} script verification)…",
        if args.skip_script { "skipping" } else { "with" }
    );
    let mut ctx = ReplayCtx {
        storage,
        skip_script: args.skip_script,
        cache: ScriptExecutionCache::new(SCRIPT_CACHE_ENTRIES),
        token: u64::from(tip.height.0).saturating_add(1),
    };
    let mut reporter = Reporter::new(start_height, stop_height);
    let mut last_committed = tip.height;

    for (height, header) in &header_pairs {
        replay_one_block(*height, *header, &fetcher, &snapshot, &mut ctx)?;
        last_committed = *height;
        reporter.tick(*height);
    }

    reporter.done(last_committed);
    ctx.storage
        .shutdown()
        .map_err(|e| format!("shutdown storage: {e}"))?;
    Ok(())
}

fn resolve_headers_and_blocks(
    args: &Args,
    start_height: BlockHeight,
    tip_hash: Hash256,
    fetcher: &mut AnyFetcher,
) -> Result<ResolvedRange, String> {
    let (cache_start, cached_headers) = match try_load_headers(&args.data_dir) {
        Ok(Some((s, h))) => (Some(s), h),
        Ok(None) => (None, Vec::new()),
        Err(e) => return Err(format!("load headers.bin: {e}")),
    };

    let cached_end = cache_start.map(|s| {
        BlockHeight(
            s.0.saturating_add(u32::try_from(cached_headers.len()).unwrap_or(u32::MAX))
                .saturating_sub(1),
        )
    });
    let stop_height = match args.stop_height {
        Some(h) => h,
        None => {
            if let Some(end) = cached_end
                && end >= start_height
            {
                end
            } else {
                BlockHeight(start_height.0.saturating_add(HEADERS_PER_PAGE - 1))
            }
        }
    };

    eprintln!(
        "  target range: heights {}..={} ({} blocks)",
        start_height.0,
        stop_height.0,
        u64::from(stop_height.0)
            .saturating_sub(u64::from(start_height.0))
            .saturating_add(1),
    );

    let headers: Vec<BlockHeader> =
        if cache_start == Some(BlockHeight(1)) && cached_end.is_some_and(|e| e >= stop_height) {
            let cstart = cache_start.expect("checked above").0;
            let lo = (start_height.0 - cstart) as usize;
            let hi = (stop_height.0 - cstart) as usize + 1;
            cached_headers[lo..hi].to_vec()
        } else {
            let locator = if start_height.0 == 1 {
                args.network.genesis_hash()
            } else {
                tip_hash
            };
            eprintln!(
                "  syncing headers from height {} (locator {})…",
                start_height.0,
                locator.to_display_hex(),
            );
            let pairs = fetcher.sync_headers(start_height, stop_height, locator)?;
            if pairs.is_empty() {
                return Err("peer returned 0 headers".to_string());
            }
            pairs.into_iter().map(|(_, h)| h).collect()
        };

    let header_pairs: Vec<(BlockHeight, BlockHeader)> = headers
        .iter()
        .enumerate()
        .map(|(i, h)| {
            (
                BlockHeight(start_height.0 + u32::try_from(i).expect("i fits u32")),
                *h,
            )
        })
        .collect();
    eprintln!("  downloading any missing blocks…");
    fetcher.fetch_blocks(&header_pairs)?;

    Ok((start_height, stop_height, header_pairs))
}

fn replay_one_block(
    height: BlockHeight,
    header: BlockHeader,
    fetcher: &AnyFetcher,
    snapshot: &HeaderSnapshot,
    ctx: &mut ReplayCtx,
) -> Result<(), String> {
    let bytes = fetcher.read_block_bytes(height)?;

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

    let context_view = PrefetchedView::new(
        PrehashedMap::with_hasher(BuildPrehashedHasher),
        snapshot,
        ctx.storage.tip(),
    );
    validate_block_context(&block, &context_view)
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

