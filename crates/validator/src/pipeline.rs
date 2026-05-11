use mantle::collections::{BuildPrehashedHasher, PrehashedMap, PrehashedSet};
use mantle::primitives::{Block, BlockHeader, BlockHeight, BlockRef, Hash256, OutPoint, TxHash};
use mantle_consensus::chain_view::{Coin, ConnectResult, ConnectScratch};
use mantle_consensus::error::ValidationError;
use mantle_consensus::script::ScriptExecutionCache;
use mantle_consensus::validation::connect_block;
use mantle_storage::ops::StorageOps;
use mantle_storage::types::{CommitBlockReq, CommittedResp, Durability, ReqToken, StorageError};

use crate::prefetched_view::{HeaderSnapshot, PrefetchedView};

/// Reusable per-block working buffers for the replay loop.
///
/// Created once before the loop, passed as `&mut` to
/// [`validate_and_commit_ref_reusing`]. All inner collections are `clear()`'d
/// at the start of each call but retain their backing capacity, eliminating
/// the ~5 MB/block allocation churn that dominates the mid-chain profile.
pub struct ReplayBufs {
    spent_set: PrehashedSet<OutPoint>,
    txids: Vec<Hash256>,
    hash_scratch: Vec<u8>,
    intra: PrehashedMap<OutPoint, Option<Coin>>,
    spend_ops: Vec<OutPoint>,
    bip30_ops: Vec<OutPoint>,
    lookup: Vec<mantle_storage::types::LookupResult>,
    coins: PrehashedMap<OutPoint, Option<Coin>>,
    txid_scratch: Vec<u8>,
    connect_scratch: ConnectScratch,
}

impl ReplayBufs {
    #[must_use]
    pub fn new() -> Self {
        Self {
            spent_set: PrehashedSet::with_hasher(BuildPrehashedHasher),
            txids: Vec::new(),
            hash_scratch: Vec::new(),
            intra: PrehashedMap::with_hasher(BuildPrehashedHasher),
            spend_ops: Vec::new(),
            bip30_ops: Vec::new(),
            lookup: Vec::new(),
            coins: PrehashedMap::with_hasher(BuildPrehashedHasher),
            txid_scratch: Vec::new(),
            connect_scratch: ConnectScratch::new(),
        }
    }

    fn clear(&mut self) {
        self.spent_set.clear();
        self.txids.clear();
        // hash_scratch / txid_scratch capacity retained; cleared internally on use
        self.intra.clear();
        self.spend_ops.clear();
        self.bip30_ops.clear();
        self.coins.clear();
    }
}

impl Default for ReplayBufs {
    fn default() -> Self {
        Self::new()
    }
}

/// Combined error from validation or storage.
#[derive(Debug)]
pub enum ValidationOrStorageError {
    Validation(ValidationError),
    Storage(StorageError),
}

impl std::fmt::Display for ValidationOrStorageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Validation(e) => write!(f, "validation: {e}"),
            Self::Storage(e) => write!(f, "storage: {e}"),
        }
    }
}

impl std::error::Error for ValidationOrStorageError {}

impl From<ValidationError> for ValidationOrStorageError {
    fn from(e: ValidationError) -> Self {
        Self::Validation(e)
    }
}

impl From<StorageError> for ValidationOrStorageError {
    fn from(e: StorageError) -> Self {
        Self::Storage(e)
    }
}

/// Extension trait for `Block` to collect intra-block UTXO dependencies.
trait BlockExt {
    /// Returns a map of outpoint → coin for all outputs created *within* this
    /// block that are also spent within the same block.
    fn intra_block_outputs(&self) -> PrehashedMap<OutPoint, Option<Coin>>;
}

impl BlockExt for Block {
    fn intra_block_outputs(&self) -> PrehashedMap<OutPoint, Option<Coin>> {
        let mut spent: PrehashedSet<OutPoint> = PrehashedSet::with_hasher(BuildPrehashedHasher);
        for tx in self.transactions.iter().skip(1) {
            for inp in &tx.inputs {
                spent.insert(inp.previous_output);
            }
        }

        let mut map: PrehashedMap<OutPoint, Option<Coin>> =
            PrehashedMap::with_capacity_and_hasher(spent.len(), BuildPrehashedHasher);
        let mut scratch = Vec::new();

        for (tx_index, tx) in self.transactions.iter().enumerate() {
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
                if spent.contains(&op) {
                    map.insert(
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

        map
    }
}

/// Validate a block and commit it to storage in one call.
///
/// 1. Collects all outpoints the block spends (minus intra-block deps).
/// 2. Batch-fetches them from storage.
/// 3. Re-inserts intra-block outputs so `connect_block` can look them up.
/// 4. Calls `connect_block`.
/// 5. Commits the resulting UTXO delta.
///
/// # Errors
///
/// Returns `ValidationOrStorageError` on any validation or I/O failure.
/// Validate a block and commit it to storage in one call.
///
/// Works with any `StorageHandle` variant (Arc-backed `'static` or
/// scoped-borrowed `'req`) via the `StorageOps` trait.
///
/// # Errors
///
/// Returns `ValidationOrStorageError` on any validation or I/O failure.
#[allow(clippy::too_many_arguments)]
pub fn validate_and_commit<'blk, S>(
    block: &'blk Block,
    height: BlockHeight,
    storage: &mut S,
    headers: &HeaderSnapshot,
    skip_script: bool,
    exec_cache: Option<&mut ScriptExecutionCache>,
    durability: Durability,
    token: ReqToken,
) -> Result<CommittedResp, ValidationOrStorageError>
where
    S: for<'x> StorageOps<'x>,
{
    let intra_block = block.intra_block_outputs();

    let spend_outpoints: Vec<OutPoint> = block
        .transactions
        .iter()
        .skip(1)
        .flat_map(|tx| tx.inputs.iter())
        .map(|inp| inp.previous_output)
        .filter(|op| !intra_block.contains_key(op))
        .collect();

    let mut spend_set: PrehashedSet<OutPoint> =
        PrehashedSet::with_capacity_and_hasher(spend_outpoints.len(), BuildPrehashedHasher);
    spend_set.extend(spend_outpoints.iter().copied());
    let mut scratch = Vec::new();
    let bip30_outpoints: Vec<OutPoint> = block
        .transactions
        .iter()
        .filter_map(|tx| {
            tx.txid_into(&mut scratch)
                .ok()
                .map(|txid| OutPoint { txid, vout: 0 })
        })
        .filter(|op| !intra_block.contains_key(op) && !spend_set.contains(op))
        .collect();

    let mut all_outpoints = spend_outpoints;
    all_outpoints.extend_from_slice(&bip30_outpoints);

    let mut lookup = Vec::new();
    storage.send_lookup(&all_outpoints, &mut lookup)?;
    let hash = block.header.hash();
    let tip = storage.tip();
    storage.recv_lookup()?;

    let mut coins: PrehashedMap<OutPoint, Option<Coin>> = PrehashedMap::with_capacity_and_hasher(
        all_outpoints.len() + intra_block.len(),
        BuildPrehashedHasher,
    );
    for r in lookup.drain(..) {
        coins.insert(r.outpoint, r.coin);
    }
    coins.extend(intra_block);

    let view = PrefetchedView::new(&coins, headers, tip);

    scratch.clear();
    let result = connect_block(
        block,
        height,
        &view,
        &mut scratch,
        &mut ConnectScratch::new(),
        skip_script,
        exec_cache,
        None,
    )?;

    let resp = storage.commit_block_borrowed(CommitBlockReq {
        token,
        height,
        hash,
        prev_hash: block.header.prev_block,
        header: block.header,
        created: &result.created,
        spent: &result.spent,
        block_bytes: None,
        durability,
    })?;

    Ok(resp)
}

/// Prefetch coins, validate, and connect a `BlockRef` against the UTXO set.
///
/// Returns a `ConnectResult` whose `NewCoin` entries borrow `script_pubkey`
/// directly from the block's wire bytes — zero copy. The caller is responsible
/// for passing the result to `commit_connect_result` while the wire buffer
/// (and therefore `block_ref`) is still alive.
///
/// Uses caller-owned `ReplayBufs` so hashmap/vec capacity is reused across blocks.
///
/// # Errors
///
/// Returns `ValidationOrStorageError` on any validation, lookup, or I/O failure.
///
/// # Panics
///
/// Panics if a transaction's txid cannot be hashed (infallible in practice).
#[allow(clippy::too_many_arguments)]
pub fn prefetch_and_connect<'blk, S>(
    block_ref: &'blk BlockRef<'blk>,
    height: BlockHeight,
    storage: &mut S,
    headers: &HeaderSnapshot,
    skip_script: bool,
    exec_cache: Option<&mut ScriptExecutionCache>,
    bufs: &mut ReplayBufs,
) -> Result<ConnectResult<'blk>, ValidationOrStorageError>
where
    S: for<'x> StorageOps<'x>,
{
    bufs.clear();

    for tx in block_ref.transactions().skip(1) {
        for inp in tx.inputs() {
            let Ok(i) = inp else { continue };
            bufs.spent_set.insert(i.previous_output);
        }
    }

    for tx in block_ref.transactions() {
        bufs.txids.push(
            tx.txid_into(&mut bufs.hash_scratch)
                .expect("txid hash never fails"),
        );
    }

    for (tx_index, tx) in block_ref.transactions().enumerate() {
        let txid = bufs.txids[tx_index];
        for (vout, output) in tx.outputs().enumerate() {
            let Ok(out) = output else { continue };
            let Ok(vout_u32) = u32::try_from(vout) else {
                continue;
            };
            let op = OutPoint {
                txid,
                vout: vout_u32,
            };
            if bufs.spent_set.contains(&op) {
                bufs.intra.insert(
                    op,
                    Some(Coin {
                        value: out.value,
                        script_pubkey: out.script_pubkey.into(),
                        height: BlockHeight::ZERO,
                        coinbase: tx_index == 0,
                    }),
                );
            }
        }
    }

    for tx in block_ref.transactions().skip(1) {
        for inp in tx.inputs() {
            let Ok(i) = inp else { continue };
            if !bufs.intra.contains_key(&i.previous_output) {
                bufs.spend_ops.push(i.previous_output);
            }
        }
    }

    bufs.spent_set.clear();
    bufs.spent_set.extend(bufs.spend_ops.iter().copied());
    for &txid in &bufs.txids {
        let op = OutPoint { txid, vout: 0 };
        if !bufs.intra.contains_key(&op) && !bufs.spent_set.contains(&op) {
            bufs.bip30_ops.push(op);
        }
    }

    let all_ops_len = bufs.spend_ops.len() + bufs.bip30_ops.len();
    bufs.spend_ops.extend_from_slice(&bufs.bip30_ops);

    storage.send_lookup(&bufs.spend_ops, &mut bufs.lookup)?;
    let tip = storage.tip();
    storage.recv_lookup()?;

    bufs.coins.reserve(all_ops_len + bufs.intra.len());
    for r in bufs.lookup.drain(..) {
        bufs.coins.insert(r.outpoint, r.coin);
    }
    for (k, v) in bufs.intra.drain() {
        bufs.coins.insert(k, v);
    }

    let view = PrefetchedView::new(&bufs.coins, headers, tip);
    let result = connect_block(
        block_ref,
        height,
        &view,
        &mut bufs.txid_scratch,
        &mut bufs.connect_scratch,
        skip_script,
        exec_cache,
        Some(&bufs.txids),
    )?;

    Ok(result)
}

/// Commit a validated block's UTXO delta to storage.
///
/// Takes the `ConnectResult` from [`prefetch_and_connect`] and pushes it
/// through the storage ring. Must be called while the wire buffer that
/// the `ConnectResult` borrows from is still alive.
///
/// # Errors
///
/// Returns `StorageError` on ring or I/O failure.
#[allow(clippy::too_many_arguments)]
pub fn commit_connect_result<'blk, S>(
    result: &'blk ConnectResult<'blk>,
    height: BlockHeight,
    hash: Hash256,
    prev_hash: Hash256,
    header: BlockHeader,
    durability: Durability,
    token: ReqToken,
    storage: &mut S,
) -> Result<CommittedResp, StorageError>
where
    S: StorageOps<'blk>,
{
    storage.commit_block_borrowed(CommitBlockReq {
        token,
        height,
        hash,
        prev_hash,
        header,
        created: &result.created,
        spent: &result.spent,
        block_bytes: None,
        durability,
    })
}
