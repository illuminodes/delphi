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

use mantle::codec::Decode;
use mantle::collections::{BuildPrehashedHasher, PrehashedMap};
use mantle::primitives::{Block, BlockHeader, BlockHeight, Hash256, OutPoint};
use mantle_consensus::chain_view::{ChainView, Coin, TipInfo};
use mantle_consensus::error::ChainViewError;
use mantle_consensus::script::ScriptExecutionCache;
use mantle_consensus::validation::connect_block;
use mantle_storage::StorageHandle;
use mantle_storage::types::{CommitBlockReq, Durability, LookupResult, ReqToken};
use mantle_validator::prefetched_view::HeaderSnapshot;

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures")
}

// ── fixture loading ───────────────────────────────────────────────────────────

fn load_metadata(fixtures: &Path) -> u32 {
    let path = fixtures.join("metadata.bin");
    match std::fs::read(&path) {
        Ok(b) if b.len() >= 4 => u32::from_le_bytes([b[0], b[1], b[2], b[3]]),
        _ => 1, // no metadata → assume early range starting at block 1
    }
}

fn load_headers(fixtures: &Path) -> Vec<BlockHeader> {
    let bytes = std::fs::read(fixtures.join("headers.bin"))
        .expect("fixtures/headers.bin not found — run fetch-fixtures first");
    assert!(
        bytes.len() % 80 == 0,
        "headers.bin size not a multiple of 80"
    );
    bytes
        .chunks_exact(80)
        .map(|chunk| {
            BlockHeader::from_bytes(chunk.try_into().expect("chunk is 80 bytes"))
                .expect("valid header bytes")
        })
        .collect()
}

fn load_block(fixtures: &Path, height: u32) -> Block {
    let path = fixtures.join("blocks").join(format!("{height:07}.bin"));
    let bytes = std::fs::read(&path)
        .unwrap_or_else(|_| panic!("{} not found — run fetch-fixtures first", path.display()));
    Block::decode(&mut std::io::Cursor::new(&bytes))
        .unwrap_or_else(|e| panic!("decode block {height}: {e}"))
}

// ── storage helpers ───────────────────────────────────────────────────────────

struct TmpDir(PathBuf);
impl Drop for TmpDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn tmp_storage() -> (StorageHandle, TmpDir) {
    let ns = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .subsec_nanos();
    let path = std::env::temp_dir().join(format!(
        "mantle-replay-mainnet-{}-{}",
        std::process::id(),
        ns
    ));
    std::fs::create_dir_all(&path).expect("create tmp dir");
    let handle = StorageHandle::open(&path).expect("StorageHandle::open");
    (handle, TmpDir(path))
}

// ── HeaderSnapshot construction ───────────────────────────────────────────────

/// Build a snapshot for ranges starting at block 1 (genesis era).
/// Inserts a synthetic pre-genesis slot at height 0 so height-1 lookups work.
fn make_snapshot_from_genesis(headers: &[BlockHeader]) -> HeaderSnapshot {
    let mut entries: Vec<(Hash256, BlockHeader)> = Vec::with_capacity(headers.len() + 1);
    let genesis_hash = headers[0].prev_block;
    entries.push((genesis_hash, headers[0]));
    for hdr in headers {
        entries.push((hdr.hash(), *hdr));
    }
    HeaderSnapshot::new(BlockHeight(0), entries).expect("snapshot fits u32")
}

/// Build a snapshot for a mid-chain range.
/// The snapshot only covers start..=end; header lookups outside this window
/// return None (which is fine since we skip script and header-context validation).
fn make_snapshot_mid_chain(headers: &[BlockHeader], start: u32) -> HeaderSnapshot {
    let entries: Vec<(Hash256, BlockHeader)> =
        headers.iter().map(|hdr| (hdr.hash(), *hdr)).collect();
    HeaderSnapshot::new(BlockHeight(start), entries).expect("snapshot fits u32")
}

// ── BenchView: PrefetchedView with dummy-coin fallback for mid-chain ──────────

/// A `ChainView` that serves prefetched coins and falls back to a dummy coin
/// for unknown outpoints. Used when replaying mid-chain blocks where we don't
/// have the full historical UTXO set — lets us measure throughput on real block
/// data without a `MissingInput` error stopping the bench.
struct BenchView<'a> {
    coins: PrehashedMap<OutPoint, Option<Coin>>,
    headers: &'a HeaderSnapshot,
    tip: TipInfo,
    dummy_on_miss: bool,
    dummy: Coin,
}

impl<'a> BenchView<'a> {
    fn new(
        coins: PrehashedMap<OutPoint, Option<Coin>>,
        headers: &'a HeaderSnapshot,
        tip: TipInfo,
        dummy_on_miss: bool,
    ) -> Self {
        Self {
            coins,
            headers,
            tip,
            dummy_on_miss,
            dummy: Coin {
                value: mantle::primitives::Satoshi::new(2_100_000_000_000_000),
                script_pubkey: vec![0x51].into(),
                height: BlockHeight(0),
                coinbase: false,
            },
        }
    }
}

impl ChainView for BenchView<'_> {
    fn get_coin(&self, outpoint: &OutPoint) -> Result<Option<&Coin>, ChainViewError> {
        match self.coins.get(outpoint) {
            Some(Some(c)) => Ok(Some(c)),
            Some(None) | None if self.dummy_on_miss => Ok(Some(&self.dummy)),
            Some(None) => Ok(None),
            None => Err(ChainViewError::NotFound),
        }
    }

    fn get_header(&self, hash: &Hash256) -> Result<Option<BlockHeader>, ChainViewError> {
        Ok(self.headers.get_header(hash))
    }

    fn get_header_at(&self, height: BlockHeight) -> Result<Option<BlockHeader>, ChainViewError> {
        Ok(self.headers.get_header_at(height))
    }

    fn height_of(&self, hash: &Hash256) -> Result<Option<BlockHeight>, ChainViewError> {
        Ok(self.headers.height_of(hash))
    }

    fn median_time_past(&self, header: &BlockHeader) -> Result<u32, ChainViewError> {
        self.headers
            .median_time_past(header)
            .ok_or(ChainViewError::NotFound)
    }

    fn tip(&self) -> TipInfo {
        self.tip.clone()
    }
}

// ── pipeline (mirrors validate_and_commit but uses BenchView) ─────────────────

fn replay_block(
    block: &Block,
    height: BlockHeight,
    storage: &mut StorageHandle,
    snap: &HeaderSnapshot,
    cache: &mut ScriptExecutionCache,
    token: u64,
    dummy_on_miss: bool,
) {
    use mantle::collections::PrehashedSet;
    use mantle::primitives::TxHash as _;

    // Intra-block outputs.
    let mut spent_set: PrehashedSet<OutPoint> = PrehashedSet::default();
    for tx in block.transactions.iter().skip(1) {
        for inp in &tx.inputs {
            spent_set.insert(inp.previous_output);
        }
    }
    let mut intra: PrehashedMap<OutPoint, Option<Coin>> =
        PrehashedMap::with_capacity_and_hasher(spent_set.len(), BuildPrehashedHasher);
    let mut scratch = Vec::new();
    for (tx_index, tx) in block.transactions.iter().enumerate() {
        let Ok(txid) = tx.txid_into(&mut scratch) else {
            continue;
        };
        for (vout, output) in tx.outputs.iter().enumerate() {
            let Ok(vout_u32) = u32::try_from(vout) else {
                continue;
            };
            let op = OutPoint {
                txid,
                vout: vout_u32,
            };
            if spent_set.contains(&op) {
                intra.insert(
                    op,
                    Some(Coin {
                        value: output.value,
                        script_pubkey: output.script_pubkey.clone(),
                        height: BlockHeight::ZERO,
                        coinbase: tx_index == 0,
                    }),
                );
            }
        }
    }

    // External spend outpoints.
    let spend_ops: Vec<OutPoint> = block
        .transactions
        .iter()
        .skip(1)
        .flat_map(|tx| tx.inputs.iter())
        .map(|inp| inp.previous_output)
        .filter(|op| !intra.contains_key(op))
        .collect();

    // BIP30 probes.
    let spend_op_set: PrehashedSet<OutPoint> = spend_ops.iter().copied().collect();
    scratch.clear();
    let bip30_ops: Vec<OutPoint> = block
        .transactions
        .iter()
        .filter_map(|tx| {
            tx.txid_into(&mut scratch)
                .ok()
                .map(|txid| OutPoint { txid, vout: 0 })
        })
        .filter(|op| !intra.contains_key(op) && !spend_op_set.contains(op))
        .collect();

    let mut all_ops = spend_ops;
    all_ops.extend_from_slice(&bip30_ops);

    storage.send_lookup(&all_ops).expect("send_lookup");
    let hash = block.header.hash();
    let tip = storage.tip();
    let lookup: Vec<LookupResult> = storage.recv_lookup().expect("recv_lookup");

    let mut coins: PrehashedMap<OutPoint, Option<Coin>> =
        PrehashedMap::with_capacity_and_hasher(all_ops.len() + intra.len(), BuildPrehashedHasher);
    for r in lookup {
        coins.insert(r.outpoint, r.coin);
    }
    coins.extend(intra);

    let view = BenchView::new(coins, snap, tip, dummy_on_miss);

    let mut scratch2 = Vec::new();
    let result = connect_block(block, height, &view, &mut scratch2, true, Some(cache), None)
        .unwrap_or_else(|e| panic!("connect_block at height {}: {e}", height.0));

    storage
        .commit_block(CommitBlockReq {
            token: ReqToken(token),
            height,
            hash,
            prev_hash: block.header.prev_block,
            header: block.header,
            created: result.created,
            spent: result.spent,
            block_bytes: None,
            durability: Durability::Memory,
        })
        .unwrap_or_else(|e| panic!("commit_block at height {}: {e}", height.0));
}

// ── main ──────────────────────────────────────────────────────────────────────

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
    let blocks: Vec<Block> = (start_height..=end_height)
        .map(|h| load_block(&fixtures, h))
        .collect();
    println!(" done.");

    let (mut storage, _tmp) = tmp_storage();
    let mut cache = ScriptExecutionCache::new(64 * 1024);

    println!("Replaying {n_blocks} blocks...");
    let t0 = Instant::now();

    for (i, block) in blocks.iter().enumerate() {
        let height = BlockHeight(start_height + u32::try_from(i).expect("fits"));
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
