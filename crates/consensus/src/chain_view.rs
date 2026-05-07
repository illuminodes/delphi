use std::collections::HashMap;

use mantle::primitives::{BlockHeader, Hash256, OutPoint};

use crate::error::ChainViewError;

// ── Coin ──────────────────────────────────────────────────────────────────────

/// A spendable output in the UTXO set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Coin {
    pub value: u64,
    pub script_pubkey: Vec<u8>,
    /// Height at which this output was created.
    pub height: u32,
    /// True if this output was created by a coinbase transaction.
    pub coinbase: bool,
}

// ── TipInfo ───────────────────────────────────────────────────────────────────

/// Snapshot of the chain tip as seen by a `ChainView`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TipInfo {
    pub hash: Hash256,
    pub height: u32,
}

// ── ChainView ─────────────────────────────────────────────────────────────────

/// Read-only view into chain state required by the validator.
///
/// The production implementation wraps the storage-thread ring-buffer API.
/// Test implementations (`MemoryChainView`) satisfy the trait in-process with
/// no I/O. The validator is generic over this trait so neither side needs the
/// other to be present at compile time.
pub trait ChainView {
    /// Look up a single UTXO by outpoint.
    ///
    /// # Errors
    /// Returns `Err` on storage failure. Returns `Ok(None)` if the coin does
    /// not exist (spent or never created).
    fn get_coin(&self, outpoint: &OutPoint) -> Result<Option<Coin>, ChainViewError>;

    /// Batched UTXO lookup. Result order matches `outpoints` order.
    ///
    /// # Errors
    /// Returns `Err` on any storage failure. On success, each element is
    /// `Some(coin)` if present or `None` if absent.
    fn get_coins(
        &self,
        outpoints: &[OutPoint],
    ) -> Result<Vec<Option<Coin>>, ChainViewError> {
        outpoints.iter().map(|op| self.get_coin(op)).collect()
    }

    /// Header by hash.
    ///
    /// # Errors
    /// Returns `Err` on storage failure. Returns `Ok(None)` if not found.
    fn get_header(&self, hash: &Hash256) -> Result<Option<BlockHeader>, ChainViewError>;

    /// Header by height on the canonical chain.
    ///
    /// # Errors
    /// Returns `Err` on storage failure. Returns `Ok(None)` if height exceeds
    /// current tip or is otherwise absent.
    fn get_header_at(&self, height: u32) -> Result<Option<BlockHeader>, ChainViewError>;

    /// Height of a header on the canonical chain, by hash.
    ///
    /// This is the inverse of `get_header_at`. Implementations are expected
    /// to satisfy this in O(1) — Stage 2 contextual validation calls it
    /// for every block, so a linear scan would make IBD quadratic.
    ///
    /// # Errors
    /// Returns `Err` on storage failure. Returns `Ok(None)` if no header
    /// with that hash exists in the canonical chain.
    fn height_of(&self, hash: &Hash256) -> Result<Option<u32>, ChainViewError>;

    /// Median-time-past for BIP113: median of the timestamps of the 11 blocks
    /// immediately preceding `header` (or fewer if near genesis).
    ///
    /// # Errors
    /// Returns `Err` if headers cannot be retrieved.
    fn median_time_past(&self, header: &BlockHeader) -> Result<u32, ChainViewError>;

    /// Current tip of the chain as seen by this view.
    fn tip(&self) -> TipInfo;
}

// ── MemoryChainView ───────────────────────────────────────────────────────────

/// In-memory `ChainView` for tests and fuzz harnesses.
///
/// Headers are stored in a `Vec` indexed by height (canonical chain only).
/// UTXOs are stored in a `HashMap`. Neither grows without bound in tests.
#[derive(Debug, Default)]
pub struct MemoryChainView {
    /// Headers in chain order. `headers[0]` is genesis.
    headers: Vec<BlockHeader>,
    /// Hash → index into `headers`.
    header_index: HashMap<Hash256, u32>,
    /// Live UTXO set.
    utxos: HashMap<OutPoint, Coin>,
}

impl MemoryChainView {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Append a header to the canonical chain. Must be called in order.
    ///
    /// # Panics
    /// Panics if the chain grows beyond `u32::MAX` headers (impossible in practice).
    pub fn push_header(&mut self, header: BlockHeader) {
        let height = u32::try_from(self.headers.len()).expect("chain height exceeds u32");
        let hash = Hash256(mantle::hash::sha256d(&header.to_bytes()));
        self.header_index.insert(hash, height);
        self.headers.push(header);
    }

    /// Insert a UTXO into the set.
    pub fn insert_coin(&mut self, outpoint: OutPoint, coin: Coin) {
        self.utxos.insert(outpoint, coin);
    }

    /// Remove a UTXO from the set (spend it).
    pub fn spend_coin(&mut self, outpoint: &OutPoint) -> Option<Coin> {
        self.utxos.remove(outpoint)
    }
}

impl ChainView for MemoryChainView {
    fn get_coin(&self, outpoint: &OutPoint) -> Result<Option<Coin>, ChainViewError> {
        Ok(self.utxos.get(outpoint).cloned())
    }

    fn get_header(&self, hash: &Hash256) -> Result<Option<BlockHeader>, ChainViewError> {
        Ok(self
            .header_index
            .get(hash)
            .and_then(|&h| self.headers.get(h as usize))
            .copied())
    }

    fn get_header_at(&self, height: u32) -> Result<Option<BlockHeader>, ChainViewError> {
        Ok(self.headers.get(height as usize).copied())
    }

    fn height_of(&self, hash: &Hash256) -> Result<Option<u32>, ChainViewError> {
        Ok(self.header_index.get(hash).copied())
    }

    fn median_time_past(&self, header: &BlockHeader) -> Result<u32, ChainViewError> {
        // Find the height of the parent of `header`, then collect up to 11
        // ancestor timestamps ending at that parent.
        let parent_height = match self.header_index.get(&header.prev_block) {
            Some(&h) => h,
            None if header.prev_block == Hash256::ZERO => {
                // Genesis parent — MTP of genesis is genesis timestamp itself.
                return Ok(self
                    .headers
                    .first()
                    .map_or(0, |h| h.time));
            }
            None => return Err(ChainViewError::NotFound),
        };

        let count = (parent_height + 1).min(11) as usize;
        let start = (parent_height + 1).saturating_sub(11) as usize;
        let mut times: Vec<u32> = self.headers[start..=parent_height as usize]
            .iter()
            .map(|h| h.time)
            .collect();
        debug_assert_eq!(times.len(), count);
        times.sort_unstable();
        Ok(times[times.len() / 2])
    }

    fn tip(&self) -> TipInfo {
        match self.headers.last() {
            None => TipInfo {
                hash: Hash256::ZERO,
                height: 0,
            },
            Some(h) => {
                let height = u32::try_from(self.headers.len() - 1).expect("chain height exceeds u32");
                TipInfo {
                    hash: Hash256(mantle::hash::sha256d(&h.to_bytes())),
                    height,
                }
            }
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn header(time: u32, prev: Hash256) -> BlockHeader {
        BlockHeader {
            version: 1,
            prev_block: prev,
            merkle_root: Hash256::ZERO,
            time,
            bits: 0x1d00ffff,
            nonce: 0,
        }
    }

    fn push_chain(view: &mut MemoryChainView, times: &[u32]) {
        let mut prev = Hash256::ZERO;
        for &t in times {
            let h = header(t, prev);
            prev = Hash256(mantle::hash::sha256d(&h.to_bytes()));
            view.push_header(h);
        }
    }

    #[test]
    fn empty_view_tip() {
        let view = MemoryChainView::new();
        let tip = view.tip();
        assert_eq!(tip.height, 0);
        assert_eq!(tip.hash, Hash256::ZERO);
    }

    #[test]
    fn get_header_by_height() {
        let mut view = MemoryChainView::new();
        push_chain(&mut view, &[1_000, 2_000, 3_000]);
        assert_eq!(view.get_header_at(0).unwrap().unwrap().time, 1_000);
        assert_eq!(view.get_header_at(2).unwrap().unwrap().time, 3_000);
        assert!(view.get_header_at(3).unwrap().is_none());
    }

    #[test]
    fn get_header_by_hash() {
        let mut view = MemoryChainView::new();
        let h = header(9_999, Hash256::ZERO);
        view.push_header(h);
        let hash = Hash256(mantle::hash::sha256d(&h.to_bytes()));
        assert_eq!(view.get_header(&hash).unwrap().unwrap().time, 9_999);
        assert!(view.get_header(&Hash256::ZERO).unwrap().is_none());
    }

    #[test]
    fn coin_insert_lookup_spend() {
        let mut view = MemoryChainView::new();
        let op = OutPoint { txid: Hash256([1u8; 32]), vout: 0 };
        let coin = Coin {
            value: 5_000_000_000,
            script_pubkey: vec![0xac],
            height: 100,
            coinbase: false,
        };
        assert!(view.get_coin(&op).unwrap().is_none());
        view.insert_coin(op, coin.clone());
        assert_eq!(view.get_coin(&op).unwrap().unwrap(), coin);
        view.spend_coin(&op);
        assert!(view.get_coin(&op).unwrap().is_none());
    }

    #[test]
    fn mtp_single_block() {
        let mut view = MemoryChainView::new();
        push_chain(&mut view, &[1_000]);
        // Block at height 1 whose parent is genesis.
        let genesis_hash = view.tip().hash;
        let next = header(2_000, genesis_hash);
        // MTP for `next` = median of [1_000] = 1_000.
        assert_eq!(view.median_time_past(&next).unwrap(), 1_000);
    }

    #[test]
    fn mtp_eleven_blocks() {
        let mut view = MemoryChainView::new();
        // Push 11 blocks with times 1..=11.
        push_chain(&mut view, &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11]);
        let tip_hash = view.tip().hash;
        let next = header(99, tip_hash);
        // Median of [1..=11] = 6.
        assert_eq!(view.median_time_past(&next).unwrap(), 6);
    }

    #[test]
    fn mtp_more_than_eleven_uses_last_eleven() {
        let mut view = MemoryChainView::new();
        // 15 blocks with times 1..=15. MTP for next block uses heights 4..=14,
        // i.e., times [5,6,7,8,9,10,11,12,13,14,15], median = 10.
        push_chain(&mut view, &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15]);
        let tip_hash = view.tip().hash;
        let next = header(99, tip_hash);
        assert_eq!(view.median_time_past(&next).unwrap(), 10);
    }

    #[test]
    fn batched_get_coins() {
        let mut view = MemoryChainView::new();
        let op1 = OutPoint { txid: Hash256([1u8; 32]), vout: 0 };
        let op2 = OutPoint { txid: Hash256([2u8; 32]), vout: 0 };
        let coin = Coin { value: 1000, script_pubkey: vec![], height: 1, coinbase: false };
        view.insert_coin(op1, coin.clone());

        let results = view.get_coins(&[op1, op2]).unwrap();
        assert_eq!(results[0], Some(coin));
        assert_eq!(results[1], None);
    }
}
