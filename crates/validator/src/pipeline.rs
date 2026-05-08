use std::collections::{HashMap, HashSet};

use mantle::primitives::{Block, BlockHeight, OutPoint, TxHash};
use mantle_consensus::chain_view::Coin;
use mantle_consensus::error::ValidationError;
use mantle_consensus::script::ScriptExecutionCache;
use mantle_consensus::validation::connect_block;
use mantle_storage::types::{CommitBlockReq, CommittedResp, Durability, ReqToken, StorageError};
use mantle_storage::StorageHandle;

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
    fn intra_block_outputs(&self) -> HashMap<OutPoint, Option<Coin>>;
}

impl BlockExt for Block {
    fn intra_block_outputs(&self) -> HashMap<OutPoint, Option<Coin>> {
        let mut spent: HashSet<OutPoint> = HashSet::new();
        for tx in self.transactions.iter().skip(1) {
            for inp in &tx.inputs {
                spent.insert(inp.previous_output);
            }
        }

        let mut map: HashMap<OutPoint, Option<Coin>> = HashMap::new();
        let mut scratch = Vec::new();

        for (tx_index, tx) in self.transactions.iter().enumerate() {
            let Ok(txid) = tx.txid_into(&mut scratch) else { continue };
            for (vout, output) in tx.outputs.iter().enumerate() {
                let Ok(vout_u32) = u32::try_from(vout) else { continue };
                let op = OutPoint { txid, vout: vout_u32 };
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
    let external_outpoints: Vec<OutPoint> = block
        .transactions
        .iter()
        .skip(1)
        .flat_map(|tx| tx.inputs.iter())
        .map(|inp| inp.previous_output)
        .filter(|op| !intra_block.contains_key(op))
        .collect();

    let lookup = storage.lookup_coins(&external_outpoints)?;

    let coins: Vec<(OutPoint, Option<Coin>)> = lookup
        .into_iter()
        .map(|r| (r.outpoint, r.coin))
        .chain(intra_block)
        .collect();

    let tip = storage.tip();
    let view = PrefetchedView::new(coins, headers, tip);

    let mut scratch = Vec::new();
    let result = connect_block(block, height, &view, &mut scratch, skip_script, exec_cache)?;

    let hash = block.header.hash();
    let resp = storage.commit_block(CommitBlockReq {
        token,
        height,
        hash,
        prev_hash: block.header.prev_block,
        created: result.created,
        spent: result.spent,
        block_bytes: None,
        durability,
    })?;

    Ok(resp)
}
