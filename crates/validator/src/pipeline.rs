use mantle::collections::{BuildPrehashedHasher, PrehashedMap, PrehashedSet};
use mantle::primitives::{Block, BlockHeight, BlockRef, Hash256, OutPoint, TxHash};
use mantle_consensus::chain_view::Coin;
use mantle_consensus::error::ValidationError;
use mantle_consensus::script::ScriptExecutionCache;
use mantle_consensus::validation::connect_block;
use mantle_storage::StorageHandle;
use mantle_storage::types::{CommitBlockReq, CommittedResp, Durability, ReqToken, StorageError};

use crate::prefetched_view::{HeaderSnapshot, PrefetchedView};

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
#[allow(clippy::too_many_arguments)]
pub fn validate_and_commit(
    block: &Block,
    height: BlockHeight,
    storage: &mut StorageHandle,
    headers: &HeaderSnapshot,
    skip_script: bool,
    exec_cache: Option<&mut ScriptExecutionCache>,
    durability: Durability,
    token: ReqToken,
) -> Result<CommittedResp, ValidationOrStorageError> {
    let intra_block = block.intra_block_outputs();

    // Outpoints the block spends from prior blocks.
    let spend_outpoints: Vec<OutPoint> = block
        .transactions
        .iter()
        .skip(1)
        .flat_map(|tx| tx.inputs.iter())
        .map(|inp| inp.previous_output)
        .filter(|op| !intra_block.contains_key(op))
        .collect();

    // BIP30 probe outpoints: `(txid, 0)` for every tx in the block.
    // connect_block probes these to check for duplicate unspent coinbases.
    // They must be in the prefetched map even if absent (Ok(None) = no collision).
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

    // Fire the lookup request, then do CPU work while the storage thread
    // is busy fetching — overlap I/O latency with validation prep.
    storage.send_lookup(&all_outpoints)?;

    // CPU work that doesn't need the lookup result yet.
    let hash = block.header.hash();
    let tip = storage.tip();

    // Now collect the lookup response.
    let lookup = storage.recv_lookup()?;

    let mut coins: PrehashedMap<OutPoint, Option<Coin>> = PrehashedMap::with_capacity_and_hasher(
        all_outpoints.len() + intra_block.len(),
        BuildPrehashedHasher,
    );
    for r in lookup {
        coins.insert(r.outpoint, r.coin);
    }
    coins.extend(intra_block);

    let view = PrefetchedView::new(coins, headers, tip);

    scratch.clear();
    let result = connect_block(
        block,
        height,
        &view,
        &mut scratch,
        skip_script,
        exec_cache,
        None,
    )?;

    let resp = storage.commit_block(CommitBlockReq {
        token,
        height,
        hash,
        prev_hash: block.header.prev_block,
        header: block.header,
        created: result.created,
        spent: result.spent,
        block_bytes: None,
        durability,
    })?;

    Ok(resp)
}

/// Zero-copy variant of [`validate_and_commit`] that operates directly on a
/// block's wire bytes via [`BlockRef`].
///
/// `arena` is a caller-owned scratch buffer that holds non-witness preimages
/// for any segwit transactions in this block. Reuse it across calls so its
/// capacity is amortised.
///
/// # Errors
///
/// Returns `ValidationOrStorageError` on any validation or I/O failure.
///
/// # Panics
///
/// Panics if a transaction's txid cannot be hashed, which cannot happen in
/// practice because the hash algorithm is infallible.
#[allow(clippy::too_many_arguments)]
pub fn validate_and_commit_ref(
    wire: &[u8],
    arena: &mut Vec<u8>,
    height: BlockHeight,
    storage: &mut StorageHandle,
    headers: &HeaderSnapshot,
    skip_script: bool,
    exec_cache: Option<&mut ScriptExecutionCache>,
    durability: Durability,
    token: ReqToken,
) -> Result<CommittedResp, ValidationOrStorageError> {
    let block_ref = BlockRef::decode_with_arena(wire, arena).map_err(|e| {
        ValidationOrStorageError::Validation(ValidationError::MalformedTransaction {
            tx_index: usize::MAX,
        })
        .or_io(e)
    })?;

    let (intra_block, txids) = build_ref_intra_block(&block_ref);

    let (spend_outpoints, bip30_outpoints) =
        collect_ref_outpoints(&block_ref, &intra_block, &txids);
    let mut all_outpoints = spend_outpoints;
    all_outpoints.extend_from_slice(&bip30_outpoints);

    storage.send_lookup(&all_outpoints)?;

    let hash = block_ref.header.hash();
    let prev_hash = block_ref.header.prev_block;
    let tip = storage.tip();
    let lookup = storage.recv_lookup()?;

    let mut coins: PrehashedMap<OutPoint, Option<Coin>> = PrehashedMap::with_capacity_and_hasher(
        all_outpoints.len() + intra_block.len(),
        BuildPrehashedHasher,
    );
    for r in lookup {
        coins.insert(r.outpoint, r.coin);
    }
    coins.extend(intra_block);

    let view = PrefetchedView::new(coins, headers, tip);
    let mut scratch = Vec::new();
    let result = connect_block(
        &block_ref,
        height,
        &view,
        &mut scratch,
        skip_script,
        exec_cache,
        Some(&txids),
    )?;

    let resp = storage.commit_block(CommitBlockReq {
        token,
        height,
        hash,
        prev_hash,
        header: block_ref.header,
        created: result.created,
        spent: result.spent,
        block_bytes: None,
        durability,
    })?;

    Ok(resp)
}

fn build_ref_intra_block(
    block_ref: &BlockRef<'_>,
) -> (PrehashedMap<OutPoint, Option<Coin>>, Vec<Hash256>) {
    let mut spent: PrehashedSet<OutPoint> = PrehashedSet::with_hasher(BuildPrehashedHasher);
    for tx in block_ref.transactions().skip(1) {
        for inp in tx.inputs() {
            let Ok(i) = inp else { continue };
            spent.insert(i.previous_output);
        }
    }

    let mut scratch = Vec::new();
    let txids: Vec<Hash256> = block_ref
        .transactions()
        .map(|tx| tx.txid_into(&mut scratch).expect("txid hash never fails"))
        .collect();

    let mut intra_block: PrehashedMap<OutPoint, Option<Coin>> =
        PrehashedMap::with_capacity_and_hasher(spent.len(), BuildPrehashedHasher);
    for (tx_index, tx) in block_ref.transactions().enumerate() {
        let txid = txids[tx_index];
        for (vout, output) in tx.outputs().enumerate() {
            let Ok(out) = output else { continue };
            let Ok(vout_u32) = u32::try_from(vout) else {
                continue;
            };
            let op = OutPoint { txid, vout: vout_u32 };
            if spent.contains(&op) {
                intra_block.insert(
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

    (intra_block, txids)
}

fn collect_ref_outpoints(
    block_ref: &BlockRef<'_>,
    intra_block: &PrehashedMap<OutPoint, Option<Coin>>,
    txids: &[Hash256],
) -> (Vec<OutPoint>, Vec<OutPoint>) {
    let mut spend_outpoints: Vec<OutPoint> = Vec::new();
    for tx in block_ref.transactions().skip(1) {
        for inp in tx.inputs() {
            let Ok(i) = inp else { continue };
            let op = i.previous_output;
            if !intra_block.contains_key(&op) {
                spend_outpoints.push(op);
            }
        }
    }

    let mut spend_set: PrehashedSet<OutPoint> =
        PrehashedSet::with_capacity_and_hasher(spend_outpoints.len(), BuildPrehashedHasher);
    spend_set.extend(spend_outpoints.iter().copied());
    let mut bip30_outpoints: Vec<OutPoint> = Vec::with_capacity(block_ref.tx_count());
    for &txid in txids {
        let op = OutPoint { txid, vout: 0 };
        if !intra_block.contains_key(&op) && !spend_set.contains(&op) {
            bip30_outpoints.push(op);
        }
    }

    (spend_outpoints, bip30_outpoints)
}

impl ValidationOrStorageError {
    /// Combine a non-validation I/O error into a Storage variant by losing
    /// context — used at decode boundaries where the upstream error type is
    /// `std::io::Error`. Matches existing crate-internal usage of `From`.
    fn or_io(self, _e: std::io::Error) -> Self {
        // Map any decoder I/O into a structural validation failure rather than
        // surfacing the raw io::Error type here. The validation error already
        // identifies the failure mode as MalformedTransaction; the original
        // io::Error message is dropped (the validator pipeline does not log it).
        self
    }
}
