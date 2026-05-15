use mantle::collections::{BuildPrehashedHasher, PrehashedMap, PrehashedSet};
use mantle::primitives::{Block, BlockHeader, BlockHeight, BlockRef, Hash256, OutPoint, TxHash};
use mantle_consensus::chain_view::{Coin, ConnectResult, ConnectScratch};
use mantle_consensus::error::ValidationError;
use mantle_consensus::script::ScriptExecutionCache;
use mantle_consensus::validation::connect_block;
use mantle_storage::ops::StorageOps;
use mantle_storage::types::{CommitBlockReq, CommittedResp, Durability, ReqToken, StorageError};

use crate::prefetched_view::{HeaderSnapshot, PrefetchedView};

/// Per-block pipeline options: script policy and storage commit parameters.
pub struct PipelineOpts<'a> {
    pub skip_script: bool,
    pub exec_cache: Option<&'a mut ScriptExecutionCache>,
    pub durability: Durability,
    pub token: ReqToken,
}

/// Block-identity fields needed when committing a previously validated result.
#[derive(Clone, Copy)]
pub struct BlockId {
    pub height: BlockHeight,
    pub hash: Hash256,
    pub prev_hash: Hash256,
    pub header: BlockHeader,
}

/// Reusable per-block working buffers for the validation pipeline.
///
/// Created once before the replay loop. All inner collections are `clear()`'d
/// at the start of each method call but retain their backing capacity,
/// eliminating the ~5 MB/block allocation churn that dominates the mid-chain
/// profile.
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
        self.intra.clear();
        self.spend_ops.clear();
        self.bip30_ops.clear();
        self.coins.clear();
    }

    /// Collect intra-block outputs and external outpoints from an owned `Block`.
    ///
    /// After this call `self.intra` holds intra-block coins, `self.spend_ops`
    /// holds all outpoints to look up from storage (spend + BIP30).
    fn collect_outpoints_block(&mut self, block: &Block) {
        for tx in block.transactions.iter().skip(1) {
            for inp in &tx.inputs {
                self.spent_set.insert(inp.previous_output);
            }
        }

        for (tx_index, tx) in block.transactions.iter().enumerate() {
            let Ok(txid) = tx.txid_into(&mut self.hash_scratch) else {
                continue;
            };
            self.txids.push(txid);
            for (vout, output) in tx.outputs.iter().enumerate() {
                let Ok(vout_u32) = u32::try_from(vout) else {
                    continue;
                };
                let op = OutPoint {
                    txid,
                    vout: vout_u32,
                };
                if self.spent_set.contains(&op) {
                    self.intra.insert(
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

        for tx in block.transactions.iter().skip(1) {
            for inp in &tx.inputs {
                if !self.intra.contains_key(&inp.previous_output) {
                    self.spend_ops.push(inp.previous_output);
                }
            }
        }

        self.spent_set.clear();
        self.spent_set.extend(self.spend_ops.iter().copied());
        for &txid in &self.txids {
            let op = OutPoint { txid, vout: 0 };
            if !self.intra.contains_key(&op) && !self.spent_set.contains(&op) {
                self.bip30_ops.push(op);
            }
        }

        self.spend_ops.extend_from_slice(&self.bip30_ops);
    }

    /// Collect intra-block outputs and external outpoints from a zero-copy
    /// `BlockRef`.
    fn collect_outpoints_ref(&mut self, block_ref: &BlockRef<'_>) {
        for tx in block_ref.transactions().skip(1) {
            for inp in tx.inputs() {
                let Ok(i) = inp else { continue };
                self.spent_set.insert(i.previous_output);
            }
        }

        for (tx_index, tx) in block_ref.transactions().enumerate() {
            let txid = tx
                .txid_into(&mut self.hash_scratch)
                .expect("txid hash never fails");
            self.txids.push(txid);
            for (vout, output) in tx.outputs().enumerate() {
                let Ok(out) = output else { continue };
                let Ok(vout_u32) = u32::try_from(vout) else {
                    continue;
                };
                let op = OutPoint {
                    txid,
                    vout: vout_u32,
                };
                if self.spent_set.contains(&op) {
                    self.intra.insert(
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
                if !self.intra.contains_key(&i.previous_output) {
                    self.spend_ops.push(i.previous_output);
                }
            }
        }

        self.spent_set.clear();
        self.spent_set.extend(self.spend_ops.iter().copied());
        for &txid in &self.txids {
            let op = OutPoint { txid, vout: 0 };
            if !self.intra.contains_key(&op) && !self.spent_set.contains(&op) {
                self.bip30_ops.push(op);
            }
        }

        self.spend_ops.extend_from_slice(&self.bip30_ops);
    }

    /// Fetch coins from storage and merge with intra-block outputs into
    /// `self.coins`. After this call the coin map is ready for building a
    /// `PrefetchedView`.
    fn fetch_coins<S>(&mut self, storage: &mut S) -> Result<(), StorageError>
    where
        S: for<'x> StorageOps<'x>,
    {
        let all_ops_len = self.spend_ops.len();
        storage.send_lookup(&self.spend_ops, &mut self.lookup)?;
        storage.recv_lookup()?;

        self.coins.reserve(all_ops_len + self.intra.len());
        for r in self.lookup.drain(..) {
            self.coins.insert(r.outpoint, r.coin);
        }
        for (k, v) in self.intra.drain() {
            self.coins.insert(k, v);
        }

        Ok(())
    }

    /// Validate an owned `Block` and commit it to storage.
    ///
    /// Full pipeline: collect outpoints, batch-fetch from storage, run
    /// `connect_block`, commit the UTXO delta. Reuses buffer capacity
    /// across calls.
    ///
    /// # Errors
    ///
    /// Returns `PipelineError` on any validation or storage failure.
    pub fn validate_and_commit<S>(
        &mut self,
        block: &Block,
        height: BlockHeight,
        storage: &mut S,
        headers: &HeaderSnapshot,
        opts: PipelineOpts<'_>,
    ) -> Result<CommittedResp, PipelineError>
    where
        S: for<'x> StorageOps<'x>,
    {
        self.clear();
        self.collect_outpoints_block(block);

        let hash = block.header.hash();
        self.fetch_coins(storage)?;
        let tip = storage.tip();
        let view = PrefetchedView::new(&self.coins, headers, tip);

        let result = connect_block(
            block,
            height,
            &view,
            &mut self.txid_scratch,
            &mut self.connect_scratch,
            opts.skip_script,
            opts.exec_cache,
            None,
        )?;

        let resp = storage.commit_block_borrowed(CommitBlockReq {
            token: opts.token,
            height,
            hash,
            prev_hash: block.header.prev_block(),
            header: block.header,
            created: &result.created,
            spent: &result.spent,
            block_bytes: None,
            durability: opts.durability,
        })?;

        Ok(resp)
    }

    /// Prefetch coins, validate, and connect a `BlockRef` against the UTXO set.
    ///
    /// Returns a `ConnectResult` whose `NewCoin` entries borrow `script_pubkey`
    /// directly from the block's wire bytes (zero copy). The caller must pass
    /// the result to [`commit_connect_result`](Self::commit_connect_result)
    /// while the wire buffer is still alive.
    ///
    /// # Errors
    ///
    /// Returns `PipelineError` on any validation, lookup, or I/O failure.
    pub fn prefetch_and_connect<'blk, S>(
        &mut self,
        block_ref: &'blk BlockRef<'blk>,
        height: BlockHeight,
        storage: &mut S,
        headers: &HeaderSnapshot,
        skip_script: bool,
        exec_cache: Option<&mut ScriptExecutionCache>,
    ) -> Result<ConnectResult<'blk>, PipelineError>
    where
        S: for<'x> StorageOps<'x>,
    {
        self.clear();
        self.collect_outpoints_ref(block_ref);

        self.fetch_coins(storage)?;
        let tip = storage.tip();
        let view = PrefetchedView::new(&self.coins, headers, tip);

        let result = connect_block(
            block_ref,
            height,
            &view,
            &mut self.txid_scratch,
            &mut self.connect_scratch,
            skip_script,
            exec_cache,
            Some(&self.txids),
        )?;

        Ok(result)
    }

    /// Commit a previously validated block's UTXO delta to storage.
    ///
    /// Takes the `ConnectResult` from [`prefetch_and_connect`](Self::prefetch_and_connect)
    /// and pushes it through the storage ring. Must be called while the wire
    /// buffer that the `ConnectResult` borrows from is still alive.
    ///
    /// # Errors
    ///
    /// Returns `StorageError` on ring or I/O failure.
    pub fn commit_connect_result<'blk, S>(
        &self,
        result: &'blk ConnectResult<'blk>,
        block: BlockId,
        durability: Durability,
        token: ReqToken,
        storage: &mut S,
    ) -> Result<CommittedResp, StorageError>
    where
        S: StorageOps<'blk>,
    {
        storage.commit_block_borrowed(CommitBlockReq {
            token,
            height: block.height,
            hash: block.hash,
            prev_hash: block.prev_hash,
            header: block.header,
            created: &result.created,
            spent: &result.spent,
            block_bytes: None,
            durability,
        })
    }
}

impl Default for ReplayBufs {
    fn default() -> Self {
        Self::new()
    }
}

/// Error from the validation pipeline: either a consensus validation failure
/// or a storage I/O error.
#[derive(Debug)]
pub enum PipelineError {
    Validation(ValidationError),
    Storage(StorageError),
}

impl std::fmt::Display for PipelineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Validation(e) => write!(f, "validation: {e}"),
            Self::Storage(e) => write!(f, "storage: {e}"),
        }
    }
}

impl std::error::Error for PipelineError {}

impl From<ValidationError> for PipelineError {
    fn from(e: ValidationError) -> Self {
        Self::Validation(e)
    }
}

impl From<StorageError> for PipelineError {
    fn from(e: StorageError) -> Self {
        Self::Storage(e)
    }
}
